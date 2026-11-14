# Integrated Wayland validation — 2026-11-14

Tested merged `main` at `129dc36806d0f92c75dfccff98e531431a396467`
(all seven backend phases). No implementation changes were made in this audit.
A [live KDE Plasma follow-up](./wayland-kde-2026-11-15.md) subsequently tested
KWin 6.6.6 on the physical display; its results update the checklist below
without replacing the earlier GNOME/headless measurements.

## Build and regression checks

- PASS: `cargo check`.
- PASS: `cargo check --target x86_64-unknown-linux-gnu`.
- PASS: `cargo test` outside the sandbox: 789 unit tests and 15 integration
  tests passed; 17 tests ignored. Ignored tests are not counted as verified.
- PASS: `cargo clippy --all-targets -- -D warnings`.
- PASS: `cargo build` and `cargo build --release`.
- PASS: `git diff --check`.
- PASS: [merged-main CI](https://github.com/Matrix-I/litty-terminal/actions/runs/43616269319),
  including macOS and Ubuntu. macOS was checked through CI, not a local Mac.

## Live GNOME Wayland

The GNOME desktop advertised 125% scale (`preferred_scale(150)`).
The desktop tests used temporary configs and test shells. The Telex helper
restored the previous IBus engine. Tests used the rebuilt debug binary except
the performance and continuous-output input measurements, which used release.

| Area | Result and evidence |
| --- | --- |
| IME | PASS: `wayland_text_input_smoke.py --telex`; `tieengs vieetj` commits `tiếng việt` to the PTY through text-input-v3. Terminal content purpose, cursor rectangle, preedit/commit and clean exit checked. |
| Clipboard | PASS: `wayland_data_smoke.py`; clipboard and primary selection in both directions with a GTK4 peer. |
| File drops | PASS: same script; two consecutive multi-file drops, shell quoting, Unicode/spaces/quotes and Copy completion. This source is GTK4, not a file manager. |
| Launcher activation | PASS: `wayland_activation_smoke.py`; GDK supplies a token, Litty uses it and the shell does not inherit it. |
| Covered-window occlusion | PASS: fullscreen GTK cover stops an always-animated shader, releases drawables and leaves 1 residual frame request and 0 CPU ticks over the 3-second sample. |
| Hidden event handling | PASS: bell attention receives an asynchronous token while hidden; rendering resumes after uncovering. |
| Multiple windows | PASS: activation smoke opens another window and checks seat/serial and source surface in its focus request; RAM benchmark opens two windows/six tabs and cleans up. |
| Busy terminal input | PASS: Ctrl+C interrupts `yes` and closes its shell/window in 0.431 seconds, including the input helper's deliberate sleeps. This is a responsiveness check, not input-to-photon latency. |

The first Telex attempt stopped at the helper's lost-focus guard before sending
further input. A subsequent complete attempt passed. The desktop must remain
available to the tests; the failed attempt is not evidence of an IME defect.

## Release performance on the live desktop

One sample per configuration. Linux `+bench-ram` reports RSS as footprint;
these numbers do not establish the macOS closed-window `< 50 MB` invariant.
The benchmark's own window/tab budgets passed on both backends.

| Measurement | Native Wayland | X11 / XWayland |
| --- | ---: | ---: |
| RAM, 1 window / 1 tab | 77.41 MB | 78.24 MB |
| RAM, 1 window / 3 tabs | 77.64 MB | 78.46 MB |
| RAM, 2 windows / 6 tabs | 77.90 MB | 78.79 MB |
| RAM after cleanup to 1 window / 1 tab | 77.66 MB | 78.56 MB |
| Cleanup delta | +0.25 MB | +0.32 MB |
| Idle CPU, 5-second sample | 0 ticks | 0 ticks |

Idle CPU used a sleeping test shell and disabled cursor blinking; the clock
has 100 ticks/second, so this establishes no measurable CPU use in the sample,
not a guarantee about every idle configuration. Idle RSS in that separate
probe was 70,196 KiB on Wayland and 70,292 KiB on X11.

Continuous `yes` output on Wayland requested 297 frames in 5.004 seconds
(about 59.4 requests/second). This measures client frame requests, not physical
display presentation timestamps or end-to-end latency.

## Headless Linux and X11

PASS: `scripts/linux/run.sh` in the existing Debian test image, using software
Vulkan. The container mounted the checkout read-only and used separate target
and registry volumes. Its checks, Clippy, 789 unit tests, 15 integration tests,
explicit `linux_xdg_open_dispatch_smoke`, build and three desktop smoke scripts
all passed. The 17 regular ignored tests remain outside that result except for
the separately invoked xdg-open check.

- Xvfb/Openbox: wheel reporting, non-Latin-1 keysym text, closing one of two
  windows without quitting the process, and synthetic Caps/Num Lock Kitty
  modifiers. The normal smoke used the default X11 backend with no Wayland.
- Weston headless: app id, compositor-driven 1024x640 resize, frame callbacks
  and clean destruction/exit.
- Sway headless: shifted/punctuated typing, Ctrl+W delivered to the TTY,
  24 repeated keypresses followed by release, cursor-shape binding/destruction,
  150% buffer sizing, tiled/floating title-band transitions and clean exit.
- A temporary extension of the sway script checked viewport/buffer geometry
  at 100%, 125%, 150% and 200%, then moved the window between two virtual
  outputs at 200% and 125%, in both directions. All passed. This tests protocol
  geometry, not physical monitor quality or cursor alignment.
- The X11 smoke also passed with explicit `LITTY_BACKEND=x11`.
- PASS: a separate fresh container ran the X11 smoke with
  `LITTY_BACKEND=wayland`, no Wayland socket and only Xvfb available. The
  fallback opened usable X11 windows and passed the same input/window checks.

The first container run used `--pids-limit=512` and failed 15 tests, mostly with
thread/process creation errors (`EAGAIN`); its clipboard helper test also never
observed a spawned helper. Rerunning with `--pids-limit=2048`,
`LP_NUM_THREADS=2` and `RAYON_NUM_THREADS=2` passed the complete suite. This
records an environment-sensitive failure, not an implementation fix. Both
the unsandboxed host suite and merged-main CI passed independently.
The combined wrapper later stalled starting its final Xvfb before the fallback
test launched; that check was rerun successfully in a fresh `--init` container.

## KWin 6.3.6 headless follow-up

KWin ran with `--virtual`, QPainter compositing and a private D-Bus session in
a disposable Debian container. Litty used the same merged-main debug binary
and software Vulkan. No host desktop socket was mounted and no host packages
or desktop settings were changed. KWin's optional `cap_sys_nice` file
capability was removed inside the container so no extra container privileges
were needed. Breeze/Aurorae decoration plugins and the Plasma desktop were
absent; this is compositor coverage, not a complete Plasma session.

- [x] Native toplevel, Vulkan buffers and frame callbacks; no X11 fallback.
- [x] Unicode OSC title reaches KWin's window caption.
- [x] Compositor resize produces a 900x600 configure and buffer.
- [x] Maximize/restore and fullscreen/restore produce configure events.
- [x] Minimize stops an always-animated shader: 0 frame requests and 0 CPU
  ticks in the 3-second sample; unminimize restores rendering.
- [x] A bell while minimized completes the asynchronous activation-token
  request. This does not prove every focus request will be granted by KWin.
- [x] Switching between two virtual desktops stops rendering on the hidden
  desktop and returning restores it.
- [x] Startup output scales 100%, 125%, 150% and 200% advertise the expected
  fractional scale and produce matching viewport/buffer sizes: a 960x600
  logical window draws 960x600, 1200x750, 1440x900 and 1920x1200 respectively.
- [x] Clipboard and primary selection offers from `wl-copy` reach Litty.
  Payload paste/readback was not exercised.
- [x] Text-input-v3 enables on focus; compositor close exits 0 without
  Wayland protocol errors.
- [x] Live scale changes were subsequently covered on real KDE 6.6.6 at
  100–200%. In this older headless run, `kscreen-doctor` reported `Cannot find
  output with id 1`; only startup scales passed there. See the linked live
  follow-up for the physical-panel result.
- [x] The later live KDE follow-up passed keyboard/repeat, stable-keycode
  composition, Telex IME, clipboard payloads, GTK4/Dolphin drops and client
  pointer move/resize gestures through a user-approved Remote Desktop portal.
  The older headless run could not inject input with `wtype`.
- [ ] Broader KDE focus-stealing policy and human physical input acceptance;
  the later automated desktop tests do not close those checks.

The harness initially needed corrections to its Qt rectangle assignment and
fullscreen size assumption (the virtual mode is 1024x768), and foreground
clipboard providers to avoid waiting on daemon-held capture pipes. These were
test-harness corrections, not Litty changes. Raw follow-up logs and temporary
harnesses are in `/tmp/litty-kwin-validation` on the test host.

## Acceptance checklist and scope limits

- [x] Merged-main build, Rust regression suite, Clippy and diff hygiene.
- [x] Live GNOME IME, data transfer, covered-window occlusion and activation.
- [x] Weston and sway headless window/input/decoration smoke checks.
- [x] Sway headless scale changes and moves between mixed-scale outputs.
- [x] KWin headless window state, startup scales, minimize/workspace recovery.
- [x] Live KDE input/IME, clipboard/primary payloads, GTK4/Dolphin drops, client
  pointer gestures, live output scales and minimize/workspace relocation.
- [x] Linux release RAM benchmark, sampled idle CPU and continuous output.
- [x] X11 default, explicit override and unavailable-Wayland fallback.
- [ ] Full manual GNOME/KDE/sway desktop acceptance, including the remaining
  checks below; headless results do not close this item.

- [x] KDE physical session tested with synthetic portal input: Telex,
  clipboard/primary payloads, Dolphin drops, client gestures and window state.
- [ ] Full live sway IME, clipboard, drops and focus-policy acceptance;
  headless sway checks cover only their documented subset.
- [ ] Real keyboard layouts/remaps and repeat cancellation on focus loss; pointer
  wheel/touchpad behavior and cursor-theme fallback at different scales.
- [ ] Hand-dragged client title bar and resize edges, window buttons, maximize,
  fullscreen and close behavior on each compositor.
- [x] Dead keys and Compose with stable physical keycodes on live KDE.
- [ ] Candidate popup appearance and position after zoom/resize/scale changes,
  preedit reset and composition across synthetic keymap swaps. The portal
  keysym attempt did not compose; the stable-keycode tests passed separately.
- [x] Real Dolphin multi-file drops on KDE, including Unicode/quotes/spaces
  and Copy source retention. Other file-manager/compositor combinations
  remain outside this evidence.
- [x] Live KDE minimize and test-window workspace relocation stop rendering
  and recover when shown again, in addition to the prior headless results.
- [ ] GNOME/sway minimize/workspace transitions and slow compositor/GPU stalls;
  callback starvation remains advisory occlusion.
- [ ] Physical mixed-DPI monitors, cursor/pointer alignment and visual sharpness.
- [ ] GPU validation layers were unavailable on the host; multi-window functional
  success does not establish absence of Vulkan validation warnings.
- [ ] macOS GUI and closed-window memory measurements require a Mac.
- [ ] Notification click-through and activation-token forwarding to an existing
  process remain application follow-ups; backend activation tests do not
  implement those integrations.

X11 remains the default. This report does not approve switching the default
backend or mark the complete cross-compositor acceptance checklist as done.
