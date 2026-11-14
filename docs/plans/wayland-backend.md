# Native Wayland window backend — Plan

Status: backend phases 1–7 implemented as an opt-in preview, not scheduled
into a release. Split out of [v0.13.0](./0.13.0.md) on 2026-11-13.
`LITTY_BACKEND=wayland` opens native Wayland windows; X11 remains the
default pending the cross-compositor validation below. Application-level
notification click callbacks and token forwarding to an existing instance
remain separate follow-ups (see phase 7).
Source: [2026-11-11 review](../reviews/2026-11-11-linux-history-ai-graphics.md).

## Decisions to make first

1. **Client library.** `smithay-client-toolkit` 0.20 and `wayland-client`
   0.31 are already in `Cargo.lock` through `smithay-clipboard`. The
   recommendation is to depend on them directly (registry, seat, xdg-shell,
   output handling, keyboard repeat) instead of hand-writing the protocol
   the way `x11.rs` dlopens Xlib. Adding them as direct dependencies needs
   approval.
2. **Keymaps.** Wayland sends an xkb keymap; mapping keys needs
   `libxkbcommon`. Choose between dlopen (as for libX11, no build
   dependency) and the `xkbcommon` crate.
3. **Decorations.** GNOME (mutter) has no `xdg-decoration`, so Litty must
   draw its own title bar there. It already draws one on macOS
   (`titlebar`), so the plan is to reuse that rather than add
   `sctk-adwaita`.
4. **Backend choice at run time.** Use Wayland when `WAYLAND_DISPLAY` is
   set and the connection works, otherwise X11, with an override
   (`LITTY_BACKEND=x11`) for bug reports. The same binary keeps both.

## Phases

Each phase lands on its own branch (`feat/wayland-<phase>`), builds and
passes the verification in `CLAUDE.md`, and leaves X11 unchanged.

1. **Window and surface.** *Done 2026-11-13 on `feat/wayland-window`,
   opt-in with `LITTY_BACKEND=wayland` until input lands. Decisions taken:
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
2. **Input.** *Done 2026-11-14 on `feat/wayland-input`. Decisions taken:
   libxkbcommon is opened at run time through `xkbcommon-dl` (the binary
   still links only libc; without the library keys fall back to the evdev
   table, a US layout); keyboards are handled on raw `wl_keyboard`, since
   sctk's keyboard module needs its linking `xkbcommon` feature. Keys are
   named from the keysym the keymap produces, the physical key from the
   evdev table. Key repeat runs on a timerfd the event loop watches; a
   compositor that takes over repeat (`wl_keyboard` v10, `repeat_info`
   rate 0, as GNOME 49 does) sends `repeated` keys instead. Pointer
   through sctk with `wp_cursor_shape_v1` or the cursor theme. Not yet:
   dead keys and Compose, which come with text input (phase 5), and
   horizontal scrolling, which X11 does not report either.* `wl_seat` keyboard (xkb keymap, modifiers, client-side key
   repeat at the compositor's rate and delay), pointer (buttons, motion,
   axis including high-resolution scroll), and cursor shapes through
   `wp_cursor_shape_v1`, falling back to a themed cursor surface. The
   keymap rules in `src/keymap.rs` stay as they are (Linux uses
   `Ctrl+Shift`).
3. **Scale and outputs.** *Done 2026-11-14 on `feat/wayland-scale`.
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
   headless sway at 150%. Not yet: moving between outputs of different
   scales (one monitor here), and the cursor theme fallback's own scale.*
   `wp_fractional_scale_v1` and `wp_viewporter`
   (integer `wl_output.scale` as the fallback), with monitor lookup for
   `current_monitor` and refresh rates. Centering new windows does not
   apply: Wayland clients cannot place their own windows.
4. **Decorations.** *Done 2026-11-14 on `feat/wayland-decorations`.
   Decision taken (2026-11-14): Litty's own title bar everywhere, as on
   X11, so Wayland windows ask for client-side decorations; where the
   compositor frames the window anyway (sway does for tiled windows, and
   hands floating ones back) the app drops its title band, rounded
   corners and resize edges, and brings them back when the frame is
   handed back (`WindowEvent::ServerDecorations`). Moving and resizing
   use `xdg_toplevel.move` and `resize` with the seat and serial of the
   last button press on the window. Checked on headless sway (25 rows
   tiled, 23 floating, 25 again). Not yet: a move or resize dragged by
   hand (no virtual pointer here holds a button down), the compositor's
   window menu (`show_window_menu`, not on X11 either), and tiled states
   (`tiled_left` and so on) for corners and edges.* Originally:
   `xdg-decoration` where the compositor offers server side decorations; otherwise Litty's own title bar with move
   (`xdg_toplevel.move`), resize edges, and window buttons.
5. **Text input.** *Done 2026-11-14 on
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
   Not yet checked: candidate popup appearance, KDE/wlroots IME.*
   `zwp_text_input_v3`: preedit drawn as on X11 (XIM),
   commit, and the cursor rectangle for the candidate window. Checked
   with IBus Unikey (Telex) under GNOME, as was done for X11.
6. **Clipboard and drag and drop.** *Done 2026-11-14 on
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
   reproduces the live desktop checks. Not yet checked: KDE/wlroots data
   transfer and real file-manager drops (GTK4 is the tested source).*
7. **Occlusion and activation.** *Backend implemented 2026-11-14 on
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
   Not yet checked: KDE/wlroots, minimize/workspace changes, and slow/GPU-stall
   behavior. The broader backend validation below remains outstanding.

## Done means

- Every `platform::*` window function has a Wayland implementation or an
  explicit no-op with the reason (as `CLAUDE.md` asks for platform code).
- Checked by hand on GNOME (mutter) and KDE (KWin), plus a wlroots
  compositor (sway): typing including IME, resize, fractional scale 1.25, copy and
  paste with another app, multiple windows, minimize.
- Idle RAM and CPU budgets hold (`+bench-ram`, idle CPU 0%).
- `ROADMAP.md` and the [configuration guide](../guide/configuration.md)
  updated; X11 remains available and tested.
