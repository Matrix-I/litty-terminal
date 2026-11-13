# Native Wayland window backend — Plan

Status: planned, not scheduled into a release. Split out of
[v0.13.0](./0.13.0.md) on 2026-11-13. Today `NativeWindow::create`
(`src/platform/linux/native/window.rs`) opens X11 windows only, so Wayland
desktops run Litty through XWayland. That costs:

- IME under GNOME on Wayland only through a hand-started `ibus-x11`, since
  Wayland input methods use `text-input-v3`;
- fractional scaling, which XWayland renders blurry or at 2x;
- the Wayland surface (`VK_KHR_wayland_surface`) and the Wayland clipboard
  path, which exist but are never reached;
- occlusion, which under XWayland reports only minimizing
  (`_NET_WM_STATE_HIDDEN`), never a covered window.

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

1. **Window and surface.** `NativeWindow` becomes an enum over X11 and
   Wayland backends behind the existing API (`platform::*` names stay
   the same). An `xdg_toplevel` with a Vulkan surface, configure and
   resize, close, title and app id (`litty`, which matches the `.desktop`
   file), and frame callbacks driving redraws (idle CPU stays at 0%).
   Test: open, resize and close under a headless compositor (`weston
   --backend=headless`) in `scripts/linux/test.sh`.
2. **Input.** `wl_seat` keyboard (xkb keymap, modifiers, client-side key
   repeat at the compositor's rate and delay), pointer (buttons, motion,
   axis including high-resolution scroll), and cursor shapes through
   `wp_cursor_shape_v1`, falling back to a themed cursor surface. The
   keymap rules in `src/keymap.rs` stay as they are (Linux uses
   `Ctrl+Shift`).
3. **Scale and outputs.** `wp_fractional_scale_v1` and `wp_viewporter`
   (integer `wl_output.scale` as the fallback), with monitor lookup for
   `current_monitor` and refresh rates. Centering new windows does not
   apply: Wayland clients cannot place their own windows.
4. **Decorations.** `xdg-decoration` where the compositor offers server
   side decorations; otherwise Litty's own title bar with move
   (`xdg_toplevel.move`), resize edges, and window buttons.
5. **Text input.** `zwp_text_input_v3`: preedit drawn as on X11 (XIM),
   commit, and the cursor rectangle for the candidate window. Checked
   with IBus Unikey (Telex) under GNOME, as was done for X11.
6. **Clipboard and drag and drop.** The existing `smithay-clipboard` path
   on the window's own connection, plus primary selection
   (`zwp_primary_selection`) and file drops (`wl_data_device`).
7. **Occlusion and activation.** The compositor stops sending frame
   callbacks to a hidden window (minimized, covered, or on another
   workspace); `Occluded` follows them, as on macOS. `xdg_activation_v1` for
   focusing a window from a notification or a second instance.

## Done means

- Every `platform::*` window function has a Wayland implementation or an
  explicit no-op with the reason (as `CLAUDE.md` asks for platform code).
- Checked by hand on GNOME (mutter) and KDE (KWin), plus a wlroots
  compositor (sway): typing including IME, resize, fractional scale 1.25, copy and
  paste with another app, multiple windows, minimize.
- Idle RAM and CPU budgets hold (`+bench-ram`, idle CPU 0%).
- `ROADMAP.md` and the [configuration guide](../guide/configuration.md)
  updated; X11 remains available and tested.
