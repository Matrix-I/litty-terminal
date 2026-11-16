# Native Wayland window backend — Plan

Status: preview phase complete (2026-11-15, maintainer scope decision).
Backend phases 1–7 are implemented; this phase is closed at opt-in preview
scope. Not scheduled into a release. Split out of
[v0.13.0](./0.13.0.md) on 2026-11-13.
`LITTY_BACKEND=wayland` opens native Wayland windows; X11 remains the
default pending the cross-compositor validation below. Application-level
notification click callbacks and token forwarding to an existing instance
remain separate follow-ups (see phase 7).
All seven phase branches are merged in `main` at `129dc36`. The
[integrated validation report](../validation/wayland-integrated-2026-11-14.md)
records GNOME live and Weston/sway/KWin headless results. The follow-up
[live KDE report](../validation/wayland-kde-2026-11-15.md) covers Plasma 6.6.6
on the physical display, including input/IME, Dolphin drops and live scaling.

Source: [2026-11-11 review](../reviews/2026-11-11-linux-history-ai-graphics.md).

## Decisions

1. **Client library.** Use `smithay-client-toolkit` 0.20 directly, with its
   reexported `wayland-client`; retain the existing patched clipboard library.
2. **Keymaps.** Load libxkbcommon at run time through `xkbcommon-dl`.
3. **Decorations.** Use Litty's own title bar, yielding to compositor frames
   when requested. No `sctk-adwaita` dependency.
4. **Backend choice at run time.** X11 remains the default. Explicit
   `LITTY_BACKEND=wayland` opts in; an unavailable Wayland connection falls
   back to X11. Automatic Wayland selection awaits broader acceptance.

## Phases

Each phase lands on its own branch (`feat/wayland-<phase>`), builds and
passes the verification in `CLAUDE.md`, and leaves X11 unchanged.

1. [x] **Window and surface.** *Done 2026-11-13 on `feat/wayland-window`,
   opt-in with `LITTY_BACKEND=wayland` pending desktop acceptance. Decisions taken:
   sctk 0.20 as a direct dependency (no default features; its reexported
   `wayland-client`); one process-wide connection, never closed (the
   clipboard keeps its display pointer); frame callbacks requested right
   before each present (`Window::pre_present_notify`) so the present's
   commit carries them.* `NativeWindow` becomes an enum over X11 and
   Wayland backends behind the existing API (`platform::*` names stay
   the same). An `xdg_toplevel` with a Vulkan surface, configure and
   resize, close, title and app id (`litty`, which matches the `.desktop`
   file), and frame callbacks driving redraws (idle CPU stays at 0%).
   Test: open, resize and close under a headless compositor (`weston
   --backend=headless`) in `scripts/linux/test.sh`.
2. [x] **Input.** *Done 2026-11-14 on `feat/wayland-input`. Decisions taken:
   libxkbcommon is opened at run time through `xkbcommon-dl` (the binary
   still links only libc; without the library keys fall back to the evdev
   table, a US layout); keyboards are handled on raw `wl_keyboard`, since
   sctk's keyboard module needs its linking `xkbcommon` feature. Keys are
   named from the keysym the keymap produces, the physical key from the
   evdev table. Key repeat runs on a timerfd the event loop watches; a
   compositor that takes over repeat (`wl_keyboard` v10, `repeat_info`
   rate 0, as GNOME 49 does) sends `repeated` keys instead. Pointer
   through sctk with `wp_cursor_shape_v1` or the cursor theme. Dead keys
   and Compose landed in phase 5. Horizontal scrolling remains unsupported,
   as on X11.* `wl_seat` keyboard (xkb keymap, modifiers, client-side key
   repeat at the compositor's rate and delay), pointer (buttons, motion,
   axis including high-resolution scroll), and cursor shapes through
   `wp_cursor_shape_v1`, falling back to a themed cursor surface. The
   keymap rules in `src/keymap.rs` stay as they are (Linux uses
   `Ctrl+Shift`).
3. [x] **Scale and outputs.** *Done 2026-11-14 on `feat/wayland-scale`.
   Decisions taken: every surface gets a `wp_viewport` showing a buffer
   of `logical size × scale` at the logical size, so fractional and
   integer scales take the same path and the buffer never has to divide
   by the scale (the integer scale is sctk's, from
   `preferred_buffer_scale` or the outputs); without `wp_viewporter` the
   window stays at scale 1 and the compositor upscales. The viewport size
   is set right before a present, in the same commit. `current_monitor`
   is the output the surface entered last (before the first enter, the
   only output if there is one). Checked on GNOME at 125%
   (`preferred_scale(150)` before the first configure, a 968x620 window
   drawn at 1210x775, 60 Hz from the output's current mode) and on
   headless sway at 150%. Integrated validation also passed live scale
   changes from 100% to 200% and moves between virtual outputs at 200%
   and 125% on sway, plus KWin startup scales from 100% to 200%.
   Live KDE 6.6.6 output changes also passed 100–200% on the physical
   panel. Physical mixed-DPI monitor moves and cursor-theme fallback scaling
   remain unchecked.*
   `wp_fractional_scale_v1` and `wp_viewporter`
   (integer `wl_output.scale` as the fallback), with monitor lookup for
   `current_monitor` and refresh rates. Centering new windows does not
   apply: Wayland clients cannot place their own windows.
4. [x] **Decorations.** *Done 2026-11-14 on `feat/wayland-decorations`.
   Decision taken (2026-11-14): Litty's own title bar everywhere, as on
   X11, so Wayland windows ask for client-side decorations; where the
   compositor frames the window anyway (sway does for tiled windows, and
   hands floating ones back) the app drops its title band, rounded
   corners and resize edges, and brings them back when the frame is
   handed back (`WindowEvent::ServerDecorations`). Moving and resizing
   use `xdg_toplevel.move` and `resize` with the seat and serial of the
   last button press on the window. Checked on headless sway (25 rows
   tiled, 23 floating, 25 again). Live KDE also passed title-bar move and
   resize-edge gestures through portal pointer events. Physical hand input,
   the compositor's window menu (`show_window_menu`, not on X11 either), and tiled states
   (`tiled_left` and so on) for corners and edges.* Originally:
   `xdg-decoration` where the compositor offers server side decorations; otherwise Litty's own title bar with move
   (`xdg_toplevel.move`), resize edges, and window buttons.
5. [x] **Text input.** *Done 2026-11-14 on
   `feat/wayland-text-input`.
   One optional `zwp_text_input_v3` per seat, enabled only on the focused
   editable surface; preedit/commit applied atomically at `done`, reset
   on leave/disable. Candidate rectangles convert buffer pixels to
   surface-local logical pixels, including fractional scales. Content
   purpose is terminal; surrounding text/deletion is not advertised
   because the shell owns the editable line. The existing shared IME API
   draws preedit without representing its internal selection. Dead keys
   and Compose use libxkbcommon's locale table, with Ctrl/Alt/Super
   shortcuts bypassing it and focus loss resetting it. Unit tests cover
   Unicode composition, reset, shortcuts, commit/preedit ordering and
   fractional candidate rectangles. The app caches pixel geometry rather
   than just the grid cell, so zoom, scale and title-band changes refresh
   the candidate rectangle too. Checked on GNOME: text-input-v3 bind,
   enter, enable, terminal purpose, cursor rectangle, done and leave;
   IBus Unikey Telex `tieengs vieetj` arrives at the PTY as `tiếng việt`,
   with preedit/commit through v3 and a clean process exit. Reproduce with
   `/usr/bin/python3 scripts/linux/wayland_text_input_smoke.py --telex`
   in a live GNOME session (temporary input-engine selection is restored).
   Headless KWin also enables text-input-v3 on focus, without injected text.
   Live KDE also passed Unikey Telex through text-input-v3, plus dead keys
   and Compose with stable physical keycodes. A follow-up fix preserves Compose
   state across keymap replacements; KDE portal keysym injection now also
   passes (`é`, `©`), with regression tests. Candidate popup appearance and
   live wlroots IME composition remain unchecked.*
   `zwp_text_input_v3`: preedit drawn as on X11 (XIM),
   commit, and the cursor rectangle for the candidate window. Checked
   with IBus Unikey (Telex) under GNOME, as was done for X11.
6. [x] **Clipboard and drag and drop.** *Done 2026-11-14 on
   `feat/wayland-clipboard`. Clipboard and primary selection
   use `smithay-clipboard` on the window's own connection. GNOME sends
   data events to the most recently bound data device, so a local patch
   of the existing 0.7.3 dependency adds a nonblocking DnD handoff on that
   same device instead of binding a competing one. The patch and license
   are in `patches/smithay-clipboard` (see `LITTY_PATCH.md`). Drops accept
   only `text/uri-list` and Copy: local absolute file URIs are decoded,
   remote hosts and control characters rejected, and paths delivered to
   the existing shell-quoted paste path. Workers read outside the event
   loop/grid lock, bounded to four transfers, 1 MiB, 4096 files and two
   seconds; finish follows successful EOF/validation, destroy always
   follows handoff. Checked on GNOME with a GTK4 peer: clipboard and
   primary selection in both directions, plus two consecutive two-file
   drops with spaces, quotes and Vietnamese in the names (Copy completion
   and a clean exit with no protocol errors). `scripts/linux/wayland_data_smoke.py`
   reproduces the live desktop checks. Headless KWin receives clipboard and primary offers. Live KDE passed
   clipboard/primary payloads both ways with GTK4, GTK4 drops, and two
   consecutive multi-file drops from Dolphin. Live wlroots payloads/drops
   remain unchecked.*
7. [x] **Occlusion and activation.** *Backend implemented 2026-11-14 on
   `feat/wayland-activation`. A presented frame with no callback for 500 ms
   emits `Occluded(true)` once; a callback restores visibility and owes a
   redraw, even if the app stopped asking while hidden. Skipped presents
   cancel the watchdog. The event loop waits only for an outstanding
   deadline, never polls while idle/occluded. Visual animation and blink
   timers stop while occluded, including `custom-shader-animation=always`.
   This is advisory starvation detection: compositor/GPU stalls can also
   delay callbacks, and compositors need not suppress every hidden callback.
   `xdg_activation_v1` is optional; focus requests use a recent seat/serial
   and source surface, attention requests omit the input grant. Requests
   are asynchronous, deduplicated per window, and their token objects are
   destroyed on completion/cancel/window close. Wayland has no separate
   urgency-cancellation request, so `request_attention(false)` can cancel
   pending work but cannot revoke attention already shown by a compositor.
   `XDG_ACTIVATION_TOKEN` is captured/removed before threads or children
   start and consumed by the first Wayland window; the shell never inherits
   it. Compositor focus-stealing policy is always authoritative.*
   Checked on GNOME with `scripts/linux/wayland_activation_smoke.py`:
   GDK supplies a launcher token, native activation uses it, and the PTY
   environment lacks it; a fullscreen GTK cover starves callbacks, releases
   drawables and stops an always-animated shader (0–1 CPU ticks per 3 s,
   one residual frame request in the sample). Bell attention still works
   while hidden, rendering resumes on uncover, and opening another window
   from the keyboard requests a token with seat/serial and source surface.
   The process exits cleanly without protocol errors.
   **Application follow-ups:** the current `notify-send` helper has no click
   callback, and Litty has no single-instance IPC. Notification click-through
   and forwarding a token to an already-running process need those app
   integrations; the backend hooks and launcher-token path are implemented.
   Headless KWin 6.3.6 also passed minimize/workspace animation stop and
   rendering recovery, hidden bell activation and clean close. Live KDE also passed minimize, hidden bell attention and moving the
   window away from/back to the current workspace. Broader focus policy,
   GNOME/sway workspace transitions and slow/GPU-stall behavior remain open. The broader backend acceptance
   below remains outstanding.

## Acceptance checklist

Evidence: [integrated validation](../validation/wayland-integrated-2026-11-14.md)
and [live KDE follow-up](../validation/wayland-kde-2026-11-15.md).
The preview phase is complete. The checked items record existing evidence;
unchecked items are deferred beyond this phase for broader desktop acceptance
and application integration. They do not block preview completion and are not
claims of successful validation. X11 remains the default.

- [x] All seven backend phases merged; merged-main Linux/macOS CI passes.
- [x] Local checks, full Rust suite, Clippy and release build pass.
- [x] GNOME live Telex IME, clipboard/primary round trips and GTK4 file drops.
- [x] GNOME covered-window occlusion, launcher tokens and hidden attention.
- [x] Weston headless open, resize, frame callbacks and close.
- [x] Sway headless keyboard/repeat, pointer binding and decorations.
- [x] Sway live scale changes 100–200% and virtual mixed-scale output moves.
- [x] KWin headless resize, maximize/fullscreen, minimize/workspaces and close.
- [x] KWin startup scales 100–200%; text-input focus and selection offers.
- [x] Live KDE keyboard/repeat, stable-keycode dead keys/Compose and Telex IME.
- [x] Live KDE clipboard/primary round trips, GTK4 and real Dolphin drops.
- [x] Live KDE client move/resize, maximize/fullscreen, minimize/workspace
  relocation, attention, live scales 100–200%, release RAM and idle CPU.
- [x] Linux RAM benchmark budgets and sampled idle CPU; continuous-output pacing.
- [x] X11 default, override and unavailable-Wayland fallback tested.
- [x] Roadmap, configuration guide and validation/performance docs updated.
- [ ] Complete manual GNOME, KDE and sway desktop coverage: typing including
  IME, pointer/touchpad, hand move/resize, copy/paste, real file-manager drops,
  multiple windows, minimize and workspace transitions on each compositor.
- [ ] Candidate popup appearance/position after zoom/resize/scale changes;
  dead keys/Compose and preedit reset with real desktop input.
- [x] KWin live scale changes on the physical KDE panel; the older headless
  output-tool failure does not apply to this later live check.
- [ ] Physical mixed-DPI monitors and cursor fallback scaling.
- [x] Reproduce composition failure across portal keysym/keymap swaps
  (`e`/`oc` instead of `é`/`©`); stable physical-keycode composition passed.
- [x] Preserve composition across valid keymap swaps, add four regression
  tests and pass the exact KDE portal keysym reproduction twice per sequence.
  Focus/shortcut resets and missing-keymap cancellation remain tested.
- [ ] Slow/GPU-stall behavior, hardware GPU validation and macOS closed-window
  memory measurements; Linux RSS does not establish macOS footprint budgets.
- [x] Complete platform API/no-op coverage audit against the acceptance rule:
  every window function must be implemented or explicitly explain its no-op.
  Done 2026-11-16; see [v0.14.0](./0.14.0.md#wayland-acceptance-and-app-integration)
  for the one open gap (`release_free_memory`).
- [ ] Application follow-ups: notification click callbacks and token forwarding
  to an existing instance. Backend hooks do not supply these integrations.
