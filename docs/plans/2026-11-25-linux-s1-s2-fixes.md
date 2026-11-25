# Linux S1/S2 Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: subagent-driven-strict or executing-plans-strict. Tasks are `### Task N: <name>`; steps use `- [ ]`.

**Goal:** fix the root-caused S1/S2 bugs from the 2026-11-23 full test run
(`docs/validation/2026-11-23-full-test-issues.md`): ISSUE-021 (a long title
kills every window on Wayland), ISSUE-028 (F1–F12 send nothing on X11) and
ISSUE-030 (Copy leaves CLIPBOARD unchanged on X11).

**Architecture:** no ADR. Two independent changes inside
`src/platform/linux/native/`:
- Wayland: every string Litty sends that can come from outside (window title,
  activation token) is bounded so one request never passes libwayland's
  4096-byte message limit.
- X11: the key path takes the logical key from the keysym for named keys, the
  rule the Wayland path (`xkb.rs`, `Keymap::key`) already follows, by reusing
  `xkb::named_key`.

**Tech Stack:** Rust 2024, libwayland via smithay-client-toolkit 0.20, Xlib
(`XLookupString`), xkbcommon-dl keysym constants.

**Branch:** `fix/linux-s1-s2` from `main`.

## Root causes (from bugfix-strict)

| Issue | Reproduced | Root cause |
| --- | --- | --- |
| ISSUE-021 | test run 3/3 (`fc3d1d4`); code path unchanged since | `WaylandWindow::set_title` (`src/platform/linux/native/wayland.rs:738`) passes the title unbounded to `xdg_toplevel.set_title`. OSC 0/2 titles are not length-capped in the grid (`src/terminal/grid/perform.rs:392`). libwayland refuses a message over 4096 bytes (`message length 4116 exceeds 4096`) and the connection, with every window, is lost. |
| ISSUE-028 | test run, every key (`95c2158`) | X11 builds keys with `convert_linux_key` (`src/platform/linux/native/x11.rs:1176`), whose evdev table (`src/platform/linux/native/events.rs:284`) names no F-keys and no modifiers. Unnamed keys get layout text from `XLookupString`; F-keys type none, so `logical` stays `Key::Unidentified` (`events.rs`, `_ => Key::Unidentified`) and `keymap::encode` sends nothing. Wayland maps the keysym through `xkb::named_key` (`xkb.rs:280`) and works. |
| ISSUE-030 | test run, every attempt (`95c2158`) | Same gap: on X11, Shift_L / Control_L presses are `Key::Unidentified`, so `keymap::is_modifier_key` is false and `handle_key` calls `g.reset_display()` (`src/app/keyboard.rs:189`), which clears the selection (`src/terminal/grid/viewport.rs:55`). By the time C (or Insert) arrives, `A::Copy` (`src/app/actions.rs:157`) finds no selection and copies nothing. Copy-on-select is unaffected because it copies on mouse release. |

Not planned here (no root cause yet, see Scope Map): ISSUE-043 and ISSUE-050.

## Confirmed Requirements

- User asked for a fix plan for the S1/S2 issues (2026-11-25). Plan covers the
  root-caused ones; ISSUE-043 and ISSUE-050 are reported back as needing
  investigation (planning-strict: no plan on a bug without a root cause).

## Decision Log

none: no business decisions; technical choices are under Technical Defaults.

## Scope Map

Invariant 1: *no string Litty sends to the Wayland compositor makes a request
exceed 4096 bytes.*

| Path | Status |
| --- | --- |
| `xdg_toplevel.set_title` from `WaylandWindow::set_title` (OSC 0/2 titles, config `title`, 🔒 prefix, benchmark titles) | in (Task 1) |
| `xdg_toplevel.set_title` at window creation (`wayland.rs:581`, `spec.title`) | in (Task 1) |
| `xdg_activation_v1.activate(token, surface)`: tokens from `XDG_ACTIVATION_TOKEN`, D-Bus notification activation (`native/app.rs:499`), `+reuse` hand-off (`linux/activation.rs:422`); all gated by `valid_token`, which allows 4096 bytes | in (Task 2) |
| `xdg_activation_v1.activate` with a token the compositor issued (`xdg_activation_token_v1.done`, `wayland_activation.rs:97`–106), gated by `valid_token` | in (Task 2: same `valid_token` bound; tokens of 2049 to ~4070 bytes are now dropped instead of sent) |
| `xdg_activation_token_v1.set_app_id("litty")`, `xdg_toplevel.set_app_id(APP_ID)` | none: constant strings |
| text-input-v3 | none: Litty sends no surrounding text (`wayland_text_input.rs:1`), only integers |
| clipboard / DnD mime types (smithay-clipboard) | none: fixed strings from the library |
| Grid keeps an unbounded OSC 0/2 title in memory per tab | out (follow-up: cap titles in the grid like `lossy_join` caps notifications; outside this crash fix) |

Invariant 2: *on X11, a key whose keysym is a named key (function, modifier,
lock, media, navigation) reaches the app as that named key, as on Wayland.*

| Path | Status |
| --- | --- |
| X11 KeyPress/KeyRelease (`x11.rs:1164`–1195) | in (Tasks 3–4) |
| X11 IME commit (keycode 0, `ime_commit`) | none: text only, no named key |
| Wayland with keymap (`xkb.rs` `Keymap::key`) | none: already follows the rule |
| Wayland without keymap (`wayland_input.rs:175`, `convert_linux_key` only) | out (follow-up: a compositor that sends no keymap still has no F-keys; not seen in any run) |
| ISSUE-031 (Shift+Click does not extend, Shift snaps to bottom, X11) | in as verification only (Task 5): same `reset_display` on an unnamed Shift press; expected fixed by Task 4 |
| ISSUE-029 (modified F1 uses `CSI 11;n~`) | out (spec question in the issue note; keymap encoding, both backends) |
| ISSUE-043 (X11 input starves while a pane floods) | out (follow-up: investigation). 2026-11-25 re-run on `3235ce8` (headless GNOME, X11, 2 panes, `yes` in one): single pane Ctrl+C lands in 0.56 s; typing into the idle pane arrives in 1–3 s (≈1.2 s of that is injection); Ctrl+C into the flooding pane after a split could not be tested because neither a click nor Ctrl+Alt+[ moved focus in the harness, even without a flood (harness or a separate X11 bug, unknown). During the flood the main thread sat in `drm_syncobj_array_wait_timeout` (a GPU fence wait) in 35–40 of 40 samples, not in a grid-lock futex. The reported "input never arrives" did not reproduce for the idle pane; root cause unknown. |
| ISSUE-050 (keys stop after XIM server is killed) | out (unconfirmed, 1 of 1, not re-run: killing ibus rewrote the shared dconf input-source order; reproducing needs a user decision) |

## Impact Map

| Changed symbol | Caller / consumer | Before → after | Status |
| --- | --- | --- | --- |
| `WaylandWindow::set_title` (Wayland) | `app/window.rs:406` (tab title, config `title`, 🔒), `app/mod.rs:342,351` (bench titles) | any length, > ~4083 bytes kills the connection → cut to the longest prefix ≤ 2048 bytes on a char boundary | intended (Task 1 test) |
| window creation title (`wayland.rs:581`) | `WaylandWindow::create` with `spec.title` | same as above | intended (Task 1, same helper) |
| `valid_token` | `capture_startup_activation`, `offer_activation_token` (D-Bus notification, `+reuse`) | tokens 2049–4096 bytes accepted (and fatal on `activate`) → refused; the window is focused without a token, as with any refused token | intended (Task 2 test) |
| `valid_token` | `activation_wire::encode` / `decode` (`+reuse` hand-off) | a hand-off carrying a 2049–4096-byte token was accepted → the whole request is refused with `InvalidData`, as a >4096-byte token already is; the launcher then opens its own window | intended (Task 2 test via `valid_token`; real tokens are tens of bytes, the frame format and u16 length are unchanged) |
| `valid_token` | `Dispatch<XdgActivationTokenV1>::event` (`wayland_activation.rs:103`, compositor-issued token) | a 2049 to ~4070-byte compositor token was used to `activate` → dropped; the window is not activated by that request (a ~4070+ byte token already killed the connection) | intended (Task 2 test via `valid_token`; compositors issue short tokens) |
| `X11Window::lookup` (private) | the two calls in the X11 key handler | returns text → returns `(keysym, text)` | intended (Task 3, refactor; no behavior change) |
| X11 key events | `app/keyboard.rs::handle_key` | F1–F20, Shift/Ctrl/Alt/Super/Meta/Hyper, Caps/Num/Scroll Lock, Print, Pause, Menu, media keys were `Unidentified` → named. Effects: F-keys encode (`keymap::encode`); a modifier press no longer clears the selection or scrolls to the bottom, and no longer hides the mouse (`mouse_hide_while_typing`); `infer_locks` now tracks Caps/Num Lock; kitty keyboard mode reports modifier keys; user `keybind`s on F-keys match | intended (Task 4 tests; matches Wayland) |
| X11 keys the evdev table already names (Enter, Tab, arrows, …) | same | unchanged unless the layout remaps them to another named keysym (e.g. Caps Lock ↔ Escape swap), which now follows the keysym | intended (Task 4 test, remap case) |
| X11 keypad keys | same | unchanged: `keypad_key` keeps handling Num Lock (Task 4 skips the keysym rule for `KeyLocation::Numpad`) | unchanged (Task 4 `kp_home` assertion on the X11 path; `keypad_follows_num_lock` covers `convert_linux_key`) |
| X11 text keys | same | unchanged | unchanged (Task 3 test pins it) |

## Existing Data & States

- Windows already open with a long title: titles are re-sent on change, so the
  next title change sends the bounded one; no stored state. Covered by Task 1.
- Persisted data: none touched (no config keys, files, or wire format changes;
  the `+reuse` wire format is unchanged, only its accepted range narrows).

## Global Constraints

From `CLAUDE.md` (project), verbatim where it matters:
- "zero heap allocation on hot paths (PTY parsing, scrolling, render loop)":
  the title helper returns a borrowed slice; the key path allocates no more
  than it does today.
- "Never perform blocking I/O ... while holding" the grid `Mutex`: unaffected.
- Platform parity: code stays under `src/platform/linux/native/`; no new
  `pub fn` in the platform API, so `macos/` needs no counterpart.
- Bug fixes start with a failing regression test that reproduces the bug.
- Verification before every commit that touches `src/`: `cargo check`;
  `cargo check --target x86_64-unknown-linux-gnu`; `cargo test` (outside the
  sandbox); `cargo clippy --all-targets -- -D warnings`; `git diff --check`.
- Commit only when asked: tasks end with "Ready to commit", not `git commit`.
  Conventional Commits `type(scope): summary`, ≤ 72 chars, scopes `linux`.
  Author `Linh Nguyen`, no co-author trailers, stage files explicitly.
- GUI checks only in the private headless GNOME (`hs-gnome`), never the real
  desktop; give Litty a private `HOME` and `XDG_RUNTIME_DIR`; do not change
  gsettings.

## Technical Defaults

- Wire string cap 2048 bytes — the 4096-byte message also holds an 8-byte
  header, the 4-byte length, the NUL, padding and other arguments; 2048 leaves
  headroom and is far above any useful title or token.
- Truncate the title (no ellipsis) — the issue accepts "truncated or ignored";
  a prefix keeps the useful start of the title.
- Cut with `str::floor_char_boundary` (stable since Rust 1.91; toolchain here
  is 1.100, no `rust-version` in `Cargo.toml`).
- The cap lives in the Wayland backend, not the grid — the limit is a Wayland
  wire limit; X11 and macOS keep full titles.
- X11 keysym rule mirrors `Keymap::key`: the keysym under the event state
  minus Ctrl/Alt/Super (the state the X11 path already uses for text) picks the
  named key; keypad keys keep `keypad_key`.

## Risks & Rollback

| Risk | Mitigation | Revert |
| --- | --- | --- |
| A layout gives a named keysym to a key users expect to type text with on X11 | `named_key` only names non-text keysyms; same rule as Wayland, which passed the run | revert the Task 4 commit |
| Modifier presses now reach kitty-keyboard apps as key events on X11 | that is what Wayland and the kitty spec do; existing kitty encoding tests cover the encoder | revert Task 4 |
| Tokens 2049–4096 bytes refused | compositors issue short tokens; a refused token falls back to focusing without one | revert Task 2 |
| `hs-gnome` key injection drops the first event of each `drive` call | send one throw-away key (a leading space) per call, as in Task 5 | n/a |

### Task 1: Bound the Wayland window title (ISSUE-021)

Type: feature (bug fix) · Test level: unit, plus e2e in Task 5

Files:
- `src/platform/linux/native/wayland.rs` (constant and helper near the top-level
  items; `WaylandWindow::create` at line 581; `set_title` at lines 737–740;
  tests module at line 923)

Interfaces:
- Produces: `pub(super) const MAX_WIRE_STRING: usize = 2048;`
- Produces: `fn wire_string(s: &str) -> &str`

- [x] Add the failing test to the `tests` module in `wayland.rs`:

```rust
    /// A title past libwayland's 4096-byte message limit drops the whole
    /// connection (ISSUE-021), so what is sent is cut on a char boundary.
    #[test]
    fn titles_are_cut_to_fit_one_wayland_message() {
        let short = "vim ~/notes.txt";
        assert_eq!(wire_string(short), short, "a short title is sent as is");
        let long = "A".repeat(100_000);
        assert_eq!(wire_string(&long).len(), MAX_WIRE_STRING);
        // 'ệ' is 3 bytes: 2048 is not a char boundary of this string.
        let wide = "ệ".repeat(1000);
        let cut = wire_string(&wide);
        assert!(cut.len() <= MAX_WIRE_STRING && wide.starts_with(cut));
        assert_eq!(cut.len(), 2046, "the longest whole-char prefix");
    }
```

- [x] Run `cargo test titles_are_cut_to_fit_one_wayland_message`; expect
  FAIL: `cannot find function wire_string` / `cannot find value MAX_WIRE_STRING`
  (the cap does not exist, which is the bug).
- [x] Add above `impl WaylandWindow` (line 572; top-level items of `wayland.rs`):

```rust
/// libwayland refuses to send a message over 4096 bytes and the connection,
/// with every window, is lost (ISSUE-021). A string argument also costs its
/// length, NUL and padding next to the header and other arguments, so strings
/// that can come from outside (titles, activation tokens) stay under this.
pub(super) const MAX_WIRE_STRING: usize = 2048;
// Header (8) + length (4) + bytes + NUL padded to 4, plus one object argument.
const _: () = assert!(8 + 4 + (MAX_WIRE_STRING + 1).next_multiple_of(4) + 4 <= 4096);

/// The longest prefix of `s` that fits `MAX_WIRE_STRING`, cut on a char boundary.
fn wire_string(s: &str) -> &str {
    &s[..s.floor_char_boundary(MAX_WIRE_STRING)]
}
```

- [x] In `WaylandWindow::create` replace `window.set_title(spec.title);` with
  `window.set_title(wire_string(spec.title));`.
- [x] Replace the body of `set_title`:

```rust
    /// Takes effect at once: `xdg_toplevel.set_title` is not double-buffered.
    pub fn set_title(&self, title: &str) {
        self.0.window.set_title(wire_string(title));
    }
```

- [x] Run `cargo test titles_are_cut_to_fit_one_wayland_message`; expect PASS.
- [x] Run `cargo clippy --all-targets -- -D warnings`; expect no warnings.
- [x] Ready to commit: `src/platform/linux/native/wayland.rs` —
  `fix(linux): cap wayland window titles below the message limit`
  (body: ISSUE-021; any program could close every window by printing a long
  OSC 2 title; CWE-400).

### Task 2: Bound activation tokens to one Wayland message (ISSUE-021 variant)

Type: feature (bug fix) · Test level: unit

Files:
- `src/platform/linux/native/wayland_activation.rs` (line 30 `valid_token`;
  tests at lines 130–147)
- `src/platform/linux/activation_wire.rs` (comment at line 17; test at line 77)

Interfaces:
- Consumes: `super::wayland::MAX_WIRE_STRING` (Task 1)
- `pub(crate) fn valid_token(token: &str) -> bool` (signature unchanged)

- [x] Add the failing test to the `tests` module of `wayland_activation.rs`:

```rust
    /// `xdg_activation_v1.activate` carries the token plus a surface; a token
    /// that makes the request pass 4096 bytes would drop the connection.
    #[test]
    fn tokens_fit_one_activate_request() {
        assert!(valid_token(&"a".repeat(super::super::wayland::MAX_WIRE_STRING)));
        assert!(!valid_token(&"a".repeat(super::super::wayland::MAX_WIRE_STRING + 1)));
        assert!(!valid_token(&"a".repeat(4096)), "a 4096-byte token no longer fits");
    }
```

- [x] Run `cargo test tokens_fit_one_activate_request`; expect FAIL on
  the `MAX_WIRE_STRING + 1` assertion (a 2049-byte token is accepted today).
- [x] Change `valid_token`:

```rust
/// Opaque, NUL-free, and short enough that `activate` stays one Wayland message.
pub(crate) fn valid_token(token: &str) -> bool {
    !token.is_empty() && token.len() <= super::wayland::MAX_WIRE_STRING && !token.contains('\0')
}
```

- [x] In `activation_wire.rs` line 17, change the comment to
  `// \`valid_token\` bounds a token to \`MAX_WIRE_STRING\` bytes, so its length fits a u16.`
  (the existing 4097-byte refusal tests there and in `wayland_activation.rs`
  stay and still pass).
- [x] Run `cargo test activation`; expect PASS (new test, `only_valid_tokens_are_offered`,
  `launcher_tokens_are_bounded_opaque_values`, `activation_wire` tests).
- [x] Ready to commit: `src/platform/linux/native/wayland_activation.rs`,
  `src/platform/linux/activation_wire.rs` —
  `fix(linux): refuse activation tokens that overflow a wayland message`

### Task 3: Extract the X11 key translation into a pure function

Type: refactor · Test level: unit

Files:
- `src/platform/linux/native/x11.rs` (`lookup` at lines 1029–1039; key handler
  at lines 1176–1194; tests module at line 1268)

Interfaces:
- Produces: `fn x11_key_input(evdev: u32, state: u32, pressed: bool, repeat: bool, lookup: impl Fn(u32) -> (u32, Option<String>)) -> KeyInput`
- Changes (private): `fn lookup(&self, kev: &XKeyEvent, state: u32) -> (u32, Option<String>)`

- [x] Add the behavior-pinning test to the `tests` module in `x11.rs`:

```rust
    /// Text keys take the layout's text under Shift (Ctrl/Alt/Super left out)
    /// and their first level as base key; evdev-named keys keep their name.
    #[test]
    fn x11_text_keys_take_layout_text_and_named_keys_keep_their_name() {
        use crate::platform::input::NamedKey;
        const SHIFT: u32 = 1;
        const CTRL: u32 = 4;
        let layout = |state: u32| {
            assert_eq!(state & CTRL, 0, "Ctrl is left out of the lookup");
            if state & SHIFT != 0 { (0x43, Some("C".to_string())) } else { (0x63, Some("c".to_string())) }
        };
        let c = x11_key_input(46, SHIFT | CTRL, true, false, layout);
        assert_eq!(c.logical, Key::Character("C".into()));
        assert_eq!(c.text.as_deref(), Some("C"));
        assert_eq!(c.unshifted, Key::Character("c".into()));
        let home = x11_key_input(102, 0, true, false, |_| (0xff50, None));
        assert_eq!(home.logical, Key::Named(NamedKey::Home));
    }
```

- [x] Run `cargo test x11_text_keys_take_layout_text`; expect FAIL:
  `cannot find function x11_key_input`.
- [x] In `x11.rs` line 10, import `KeyInput`:
  `use crate::platform::input::{Key, KeyInput, KeyLocation, Mods};`
- [x] Add as a free function in `x11.rs` (next to `keysym_text`):

```rust
/// Ctrl, Alt and Super select no level in standard layouts; leaving them out
/// of the lookup keeps the printable character the keymap matches against.
const CTRL_ALT_SUPER: u32 = 4 | 8 | 64;

/// The key event for an X11 key press or release. `lookup(state)` is
/// `XLookupString` under `state`: the keysym and the text it types.
fn x11_key_input(evdev: u32, state: u32, pressed: bool, repeat: bool, lookup: impl Fn(u32) -> (u32, Option<String>)) -> KeyInput {
    let mut key_input = convert_linux_key(evdev, state, pressed, repeat);
    // Keys outside the evdev table (other layouts' extra keys, remapped
    // keycodes) still produce layout text, Shift and AltGr applied. This keeps
    // a layout's own keypad symbols (KP_Separator).
    if !matches!(key_input.logical, Key::Named(_)) {
        if let (_, Some(c)) = lookup(state & !CTRL_ALT_SUPER) {
            key_input.logical = Key::Character(c.clone());
            key_input.text = Some(c);
        }
        // A keypad key's base key is its Num Lock one, not the navigation
        // key `state` 0 would give.
        if key_input.location != KeyLocation::Numpad
            && let (_, Some(c)) = lookup(0)
        {
            key_input.unshifted = Key::Character(c);
        }
    }
    key_input
}
```

- [x] Change `lookup` to return the keysym too:

```rust
    /// The keysym and the text the key produces under `state`, through the X
    /// keyboard layout.
    fn lookup(&self, kev: &XKeyEvent, state: u32) -> (u32, Option<String>) {
        let mut ev = XKeyEvent { state: state as c_int, ..*kev };
        let mut buf = [0 as c_char; 16];
        let mut keysym: c_ulong = 0;
        unsafe {
            (self.0.lib.XLookupString)(&mut ev, buf.as_mut_ptr(), buf.len() as c_int, &mut keysym, std::ptr::null_mut());
        }
        // The returned bytes are Latin-1 only; the keysym covers every layout.
        (u32::try_from(keysym).unwrap_or(0), keysym_text(keysym))
    }
```

- [x] In the key handler replace lines 1176–1194 (from
  `let mut key_input = convert_linux_key(` through the closing `}` of the
  `if !matches!(...)` block) with:

```rust
                        let key_input = x11_key_input(evdev_code, state, pressed, repeat, |s| self.lookup(kev, s));
```

  and keep `events.push(WindowEvent::Key(key_input));`. Remove the now-unused
  local `const CTRL_ALT_SUPER` from the handler.
- [x] Run `cargo test x11_text_keys_take_layout_text` and
  `cargo test keypad_follows_num_lock`; expect PASS.
- [x] Run `cargo clippy --all-targets -- -D warnings`; expect no warnings
  (unused imports in the handler, if any, removed).
- [x] Ready to commit: `src/platform/linux/native/x11.rs` —
  `refactor(linux): extract x11 key translation into a pure function`

### Task 4: Name X11 keys from their keysym (ISSUE-028, ISSUE-030)

Type: feature (bug fix) · Test level: unit, plus e2e in Task 5

Files:
- `src/platform/linux/native/xkb.rs` (line 280: `fn named_key` becomes `pub(super)`)
- `src/platform/linux/native/x11.rs` (`x11_key_input` from Task 3; tests module)

Interfaces:
- Consumes: `super::xkb::named_key(sym: xkb_keysym_t) -> Option<(NamedKey, Option<KeyLocation>)>` (`xkb_keysym_t` is `u32`)
- `x11_key_input` signature unchanged

- [x] Add the failing test to the `tests` module in `x11.rs` — deviated: the committed test also covers Num Lock + Shift (state `16 | 1`, unshifted `"7"`), added in review round 1, see checkpoint 3:

```rust
    /// F-keys and modifiers are not in the evdev table; on X11 they were
    /// `Unidentified`, so F1–F12 sent nothing (ISSUE-028) and a Shift or Ctrl
    /// press cleared the selection before Copy (ISSUE-030).
    #[test]
    fn x11_function_and_modifier_keys_take_their_name_from_the_keysym() {
        use crate::platform::input::NamedKey;
        use xkbcommon_dl::keysyms as k;
        let key = |evdev: u32, sym: u32| x11_key_input(evdev, 0, true, false, move |_| (sym, None));
        assert_eq!(key(59, k::F1).logical, Key::Named(NamedKey::F1));
        assert_eq!(key(88, k::F12).logical, Key::Named(NamedKey::F12));
        let shift = key(42, k::Shift_L);
        assert_eq!((shift.logical, shift.location), (Key::Named(NamedKey::Shift), KeyLocation::Left));
        let ctrl = key(97, k::Control_R);
        assert_eq!((ctrl.logical, ctrl.location), (Key::Named(NamedKey::Control), KeyLocation::Right));
        // A remapped key means what it produces, as on Wayland: Caps Lock as Escape.
        let caps = key(58, k::Escape);
        assert_eq!((caps.logical, caps.text, caps.unshifted), (Key::Named(NamedKey::Escape), None, Key::Named(NamedKey::Escape)));
        // Ctrl+Alt+F1 is still F1: the lookup leaves Ctrl and Alt out.
        let vt = x11_key_input(59, 4 | 8, true, false, |state| {
            assert_eq!(state & (4 | 8), 0);
            (k::F1, None)
        });
        assert_eq!(vt.logical, Key::Named(NamedKey::F1));
        // Keypad keys keep the Num Lock rules of `keypad_key`.
        let kp_home = x11_key_input(71, 0, true, false, |_| (k::KP_Home, None));
        assert_eq!((kp_home.logical, kp_home.location), (Key::Named(NamedKey::Home), KeyLocation::Numpad));
    }
```

- [x] Run `cargo test x11_function_and_modifier_keys`; expect FAIL at the
  first assertion: left `Unidentified`, right `Named(F1)` — the reported
  ISSUE-028 symptom.
- [x] In `xkb.rs`, change `fn named_key(` to `pub(super) fn named_key(`.
- [x] In `x11.rs`, replace the body of `x11_key_input` with:

```rust
    let mut key_input = convert_linux_key(evdev, state, pressed, repeat);
    let (sym, text) = lookup(state & !CTRL_ALT_SUPER);
    // The keysym decides the logical key of a named key, as on Wayland
    // (`Keymap::key`): F-keys and modifiers are not in the evdev table, and a
    // remapped key (Caps Lock as Escape) means what it produces. Keypad keys
    // keep `keypad_key`'s Num Lock rules.
    if key_input.location != KeyLocation::Numpad
        && let Some((name, location)) = super::xkb::named_key(sym)
    {
        key_input.logical = Key::Named(name);
        key_input.unshifted = Key::Named(name);
        key_input.text = None;
        key_input.location = location.unwrap_or(key_input.location);
        return key_input;
    }
    // Keys outside the evdev table (other layouts' extra keys, remapped
    // keycodes) still produce layout text, Shift and AltGr applied. This keeps
    // a layout's own keypad symbols (KP_Separator).
    if !matches!(key_input.logical, Key::Named(_)) {
        if let Some(c) = text {
            key_input.logical = Key::Character(c.clone());
            key_input.text = Some(c);
        }
        // A keypad key's base key is its Num Lock one, not the navigation
        // key `state` 0 would give.
        if key_input.location != KeyLocation::Numpad
            && let (_, Some(c)) = lookup(0)
        {
            key_input.unshifted = Key::Character(c);
        }
    }
    key_input
```

- [x] Run `cargo test x11_`; expect PASS for both X11 key tests.
- [x] Run `cargo test keypad_follows_num_lock` and `cargo test keymap`;
  expect PASS (keypad and encoding unchanged).
- [x] Run `cargo clippy --all-targets -- -D warnings`; expect no warnings.
- [x] Ready to commit: `src/platform/linux/native/x11.rs`, `src/platform/linux/native/xkb.rs` —
  `fix(linux): name x11 function and modifier keys from their keysym`
  (body: ISSUE-028, ISSUE-030; same rule as the Wayland path; a modifier
  press no longer clears the selection).

### Task 5: Verification & Docs

Type: docs + verification · Test level: e2e (headless GNOME)

Files:
- `docs/validation/2026-11-23-full-test-issues.md` (ISSUE-021, 028, 030, 031,
  043 entries)

- [x] Final gate, after the last edit: read the full diff (`git diff main --stat`,
  then `git diff main`), untracked files included; nothing unrelated, no debug
  output, TODOs or commented-out code.
- [x] Run and quote results: `cargo check`; `cargo check --target x86_64-unknown-linux-gnu`;
  `cargo test` (outside the sandbox); `cargo clippy --all-targets -- -D warnings`;
  `git diff --check`. Known flaky host failure: heap-corruption abort in full
  `cargo test` with `ibus-x11` (ISSUE-014, memory "deferred failures"); if it
  appears, re-run and report it, never dismiss it.
- [x] Build: `cargo build --release`. E2E setup (all runs): `hs-gnome start`; — deviated: see checkpoint 2;
  a scratch dir `$D` with `home/` and a mode-700 `rt/`; launch with
  `eval "$(hs-gnome env)"; HOME=$D/home XDG_RUNTIME_DIR=$D/rt SHELL=$D/<script> target/release/litty &`
  (Wayland run: add `LITTY_BACKEND=wayland WAYLAND_DISPLAY=/run/user/$(id -u)/hs-gnome-0`,
  an absolute path because `XDG_RUNTIME_DIR` is private; X11 run: `WAYLAND_DISPLAY=`).
  Every `hs-gnome run python3 -m drive ...` call may lose its first event, so
  each call starts with a throw-away key (`type ' ...'`, `key space ...`) and a
  sequence that must stay together goes in one call (`drive key` takes
  several keys). A lost or kept leading space must not change the verdict.
- [x] E2E ISSUE-021 (Wayland): `$D/title.sh` =
  `#!/bin/sh` / `python3 -c "print('\x1b]2;' + 'A'*100000 + '\x07', end='')"` / `exec /bin/sh`.
  Expected: after 5 s the Litty process is alive and stderr has no
  `exceeds 4096` / `lost the connection`. Before the fix: exit 1 with both lines.
- [x] E2E ISSUE-028 (X11): `$D/keys.sh` = `#!/bin/sh` / `exec sh -c 'cat -v > "$HOME/keys.txt"'`;
  one call `drive key space F1 F5 F12 shift+F5 ctrl+F5 Return`.
  Expected `home/keys.txt` contains `^[OP^[[15~^[[24~^[[15;2~^[[15;5~`
  (optionally preceded by a space). Before: at most a space.
- [x] E2E ISSUE-030 (X11): SHELL `/bin/sh`; `drive type ' printf hello-030\n'`,
  then one call `drive key space ctrl+shift+a ctrl+shift+c`; then
  `hs-gnome run wl-paste -n` (Xwayland syncs CLIPBOARD). Expected output
  contains `hello-030`; a config with `copy-on-select = false` in
  `$D/home/.litty/config` (key parsed at `src/config.rs:482`) so
  copy-on-select cannot mask the result. Repeat with one call
  `drive key space ctrl+shift+a ctrl+Insert` after clearing CLIPBOARD
  (`hs-gnome run sh -c 'printf x | wl-copy'`).
- [x] E2E ISSUE-031 (X11): scroll up with `drive scroll`, press Shift alone; — partly fixed: see checkpoint 2;
  expected the view stays scrolled (screenshot before/after with `drive shot`).
  If it still snaps, record ISSUE-031 as open with the evidence; do not widen
  this plan.
- [x] Update `docs/validation/2026-11-23-full-test-issues.md`: under each of
  ISSUE-021, 028, 030 (and 031 if fixed) add `- Status: fixed on
  fix/linux-s1-s2 (<commit>), e2e re-run <date>: <result>`; under ISSUE-043
  add the 2026-11-25 re-run evidence from this plan's Scope Map and
  `- Status: open, needs investigation`.
- [x] README/ROADMAP: no change (no config key, keybinding or roadmap item
  changes); state this in the report.
- [x] Self-review per global `CLAUDE.md` step 3, then `code-review-strict`;
  address or report findings.
- [x] Report: commands and results, e2e evidence per issue, what was not run.
  CI: the new tests are plain unit tests and run in `cargo test` on Linux CI;
  the e2e checks are local only and the PR must say so.
- [x] Ready to commit: `docs/validation/2026-11-23-full-test-issues.md`,
  `docs/plans/2026-11-25-linux-s1-s2-fixes.md` —
  `docs(validation): record linux s1/s2 fix results`
