# Live KDE Plasma Wayland validation — 2026-11-15

Tested the installed Plasma/KWin 6.6.6 session on Ubuntu 26.04.1, with the
physical eDP-1 panel at 1920x1080/60 Hz. This is a real desktop session, not
the earlier KWin 6.3.6 virtual compositor. Checkout: `fcb242c`; application
source is unchanged from merged backend commit `129dc36`.

The native client used Vulkan and DMA-BUF buffers. Functional tests used the
debug build; performance probes used the release build. Temporary configs,
shells, files and window identities kept the probes separate from normal use.
No application source was changed.

## Passed checks

- [x] Native Wayland window, frame callbacks and Unicode OSC title.
- [x] Compositor-driven resize to 900x600, maximize/restore,
  fullscreen/restore and compositor close with exit 0.
- [x] Actual client title-bar drag: pointer events issue `xdg_toplevel.move`
  and move the window from (800,100) to (900,150).
- [x] Actual client resize-edge drag: pointer events issue
  `xdg_toplevel.resize` and grow 900x600 to 980x640.
- [x] Live output scales 125%, 150%, 200%, then 100%, with the expected
  fractional-scale events and hardware buffer sizes. Examples: 900x600 logical
  becomes 1125x750 at 125% and 1350x900 at 150%; KWin constrains the window to
  900x494 logical at 200%, which draws an 1800x988 buffer.
- [x] Keyboard text and punctuation (`Hello KDE! ~/x_1`), Ctrl+W delivered
  to the TTY, and a held key repeating 24 times then stopping on release.
- [x] Dead key and Compose through stable physical keycodes: US intl
  apostrophe + e gives `é`; Menu-as-Compose + o + c gives `©`.
  Input events are synthetic; a human-operated physical keyboard is not
  established by this result.
- [x] IBus Unikey Telex through KDE's input-method integration:
  `tieengs vieetj` arrives at the PTY as `tiếng việt`, with text-input-v3
  preedit and commit events. The installed IBus Wayland desktop entry was
  enabled temporarily for this test.
- [x] Clipboard and primary selection in both directions between GTK4 and
  Litty, including Unicode text, shortcut paste/copy and middle-button paste.
- [x] Two consecutive multi-file drops from GTK4, and two from real Dolphin.
  Paths with spaces, a quote and Vietnamese text survive shell quoting;
  source files remain in place for Copy; transfer completion is observed.
- [x] Minimize stops an always-animated shader: 0 frame requests and 0 CPU
  ticks in the 3-second sample. Bell activation requests still complete while
  hidden; unminimize restores rendering.
- [x] Moving the test window to a different virtual desktop stops rendering;
  returning it to the original desktop resumes rendering. The user's active
  desktop was not switched. A temporary second desktop was removed afterward.
- [x] The completed data, gesture/drop and window sessions exit without
  Wayland protocol errors.

Input was sent through the standard Remote Desktop portal after the user
approved its dialogs. Its temporary grant was closed after testing. KWin
scripts selected only test windows by process/window identity; this avoided
depending on GNOME's Alt+Tab order. These are automated interactions on the
real KDE session, not manual visual acceptance of every control.

## Release RAM and CPU

One sample per backend. `+bench-ram` uses RSS as footprint on Linux, and both
backends passed its window/tab budgets. These do not measure the macOS
closed-window footprint invariant.

| Measurement | Native Wayland | X11 / XWayland on KDE |
| --- | ---: | ---: |
| 1 window / 1 tab | 77.32 MB | 78.44 MB |
| 1 window / 3 tabs | 77.50 MB | 78.66 MB |
| 2 windows / 6 tabs | 77.76 MB | 78.98 MB |
| After cleanup to 1 window / 1 tab | 77.52 MB | 78.75 MB |
| Cleanup delta | +0.20 MB | +0.32 MB |
| Idle CPU, sleeping shell, cursor blink off | 0 ticks / 5 s | 0 ticks / 5 s |

The clock has 100 ticks/second. Native continuous `yes` output requested
300 frames over 5.001 seconds (~60/s); this counts client requests, not panel
presentation timestamps or input-to-photon latency.

## Initial attempts and limits

- The old GNOME helper's Alt+Tab choreography failed a focus guard on KDE.
  Selecting the two test windows explicitly allowed the data checks to pass.
- Portal keysym injection for dead_acute and Multi_key temporarily swaps
  KWin's keymap. Those sequences produced `e` and `oc`, respectively; Litty's
  compose state is rebuilt on the map changes. Stable keycode/layout tests
  passed separately. The keysym-injection case remains a limitation and is
  not counted as a successful composition test.
- Initial IME setup omitted KDE configuration notification, so IBus did not
  start. `kwriteconfig6 --notify` started it; a subsequent harness variable
  shadowing error was corrected before the complete Telex pass.
- The first scale assertion incorrectly required a new viewport-destination
  request although logical size had not changed. The destination is cached;
  the observed 1125x750 buffer was correct. The corrected geometry checks
  passed through all four live scales.
- The first Dolphin lookup used its X11-style class; native KDE uses
  `org.kde.dolphin`. Correcting the test lookup allowed both drops to pass.

Original scale 1, one desktop, the absent InputMethod setting and the original
keyboard-layout config were restored. Verification found no IBus daemon left
running and KWin reported the input method unavailable again, as initially.
Removing the temporary layout file initially left KWin's intl map cached;
the active map was explicitly reset to the system's US/no-variant/no-options
defaults before removing that file again. KDE's layout API then reported
`English (US)`, matching `/etc/default/keyboard`.
Temporary input control and KWin scripts were closed/unloaded. Raw logs and
temporary harnesses remain under `/tmp/litty-kde-live`; they are not committed.

## Still open

- [ ] Candidate popup appearance and placement after zoom/resize/scale
  changes; visual preedit selection and focus-loss/reset acceptance.
- [ ] Human physical keyboard, touchpad/high-resolution wheel, window-button
  and visual sharpness/cursor alignment checks.
- [ ] Physical mixed-DPI monitors, output hotplug and cursor-theme fallback.
- [ ] Compose sequences delivered through keysym injection and transient
  keymap swaps; stable physical keycode composition passed above.
- [ ] Focus-stealing policy and launcher-token behavior on KDE beyond the
  observed hidden attention completion; notification click-through and
  existing-instance token forwarding remain application follow-ups.
- [ ] Slow compositor/GPU stalls, hardware Vulkan validation-layer coverage,
  and full live sway IME/data-transfer acceptance.

The [integrated report](./wayland-integrated-2026-11-14.md) retains the prior
GNOME and headless results. The [backend checklist](../plans/wayland-backend.md#acceptance-checklist)
tracks overall acceptance; X11 remains the default.
