# Linux Notification Click-Through and `+reuse` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: development-superpowers:subagent-driven-development or executing-plans-strict. Steps use `- [ ]`.

**Goal:** Close the v0.14.0 deferred item "Linux notification pane click-through
and single-instance activation-token forwarding"
([0.14.0.md](./0.14.0.md#wayland-acceptance-and-app-integration),
[wayland-backend.md](./wayland-backend.md) last checklist item).

**Architecture:** Clicking a notification on Linux takes the same route as on
macOS. The click is handed to the notification handler, which passes it to a
router thread. The router sends `ActivationAction::Pane` over the instance's
activation socket. The socket server raises `AppEvent::Activation`, and the
existing `App::on_activation` / `activate_pane` code handles it. On Linux the
clicks come from `org.freedesktop.Notifications` on the session bus. XDG
activation tokens reach the Wayland backend through a process-wide one-shot
slot, and the next explicit focus uses the token in it.

**Tech Stack:** Rust 2024; libdbus-1 opened with dlopen (as `portal.rs` does);
Unix sockets; `xdg_activation_v1`.

**Branch:** `feat/linux-notification-activation`, cut from `main` (6a11ec2,
which already contains the `fix/linux-deferred-items` work). Before Task 1:
`git switch -c feat/linux-notification-activation`. The working tree has an
unrelated modification to `src/terminal/search.rs` that is not this plan's;
ask the user what to do with it before branching, and never stage it.

## Confirmed Requirements

1. Notifications go straight over D-Bus to `org.freedesktop.Notifications`.
   libdbus is loaded through dlopen; no new dependency is added. The code
   handles `ActionInvoked` (action `default`), `ActivationToken` and
   `NotificationClosed`. Litty falls back to `notify-send` when the bus cannot
   be reached or when the server answers `Notify` with an error.
   (User, 2026-11-16.)
2. **Linux only; macOS is not touched** (user, 2026-11-16: "bản này chỉ dùng
   cho linux thôi nhé không có chạy cho macos").
   - No file under `src/platform/macos/` changes.
   - The shared `src/platform/activation.rs` (LTA1 wire) does not change.
   - Shared app code changes only where a `cfg!(target_os = "macos")` gate
     has to admit Linux too. On macOS each such gate must still evaluate
     exactly as it does today.
3. Activation frames: Linux clients send `LTA2`, which is an XDG activation
   token followed by an unchanged `LTA1` request. The Linux server accepts
   both `LTA1` and `LTA2`. (User, 2026-11-16.)

## Global Constraints (project CLAUDE.md, verbatim where it matters)

- Zero heap allocation on hot paths (PTY parsing, scrolling, render loop);
  0% idle CPU (`ControlFlow::Wait`); < 50MB idle RAM budget.
- Never perform blocking I/O while holding the grid `Mutex`.
- Platform code: OS integration goes in `src/platform/{macos,linux}/`, with
  the same `pub fn` in both and exported by name from `src/platform/mod.rs`.
- Layering (`src/layering_tests.rs`): `platform` depends on nothing outside
  `crate::platform`.
- Verification before every commit that touches `src/`:
  - `cargo check`
  - `cargo check --target x86_64-unknown-linux-gnu`
  - `cargo test` (run outside the sandbox; PTY tests need `openpty`)
  - `cargo clippy --all-targets -- -D warnings`
  - `git diff --check`
- Commit only when asked. Conventional Commits, scopes `linux` and `docs`,
  imperative, lowercase, ≤ 72 chars, no trailing period. Never add a
  Co-Authored-By trailer. Stage files explicitly; never `git add -A`.
- Update `README.md` and `docs/guide/` when user-visible behavior changes,
  and `ROADMAP.md` / `0.14.0.md` when the item closes.

## Technical Defaults

- **Registry root on Linux:** `$XDG_RUNTIME_DIR/litty-a1` when that variable
  is an absolute path, otherwise `/tmp/litty-a1-<uid>`. `LITTY_ACTIVATION_DIR`
  still overrides both. Reason: the runtime dir is per-user, mode 0700, on
  tmpfs, and cleared at logout.
- **Platform calls:**
  - Peer check: `SO_PEERCRED`, because Linux has no `getpeereid`.
  - Nonce: `getrandom`, because Linux has no `arc4random_buf`.
  - `sockaddr_un` has no `sun_len` field on Linux.
- **Notification bus:** opened lazily on the first notification, as a private
  connection separate from the settings portal's. Its socket is watched as
  epoll tag 7. Reason: no extra socket or RAM until a notification is shown.
- **Signal match rule:** includes `sender='org.freedesktop.Notifications'`, so
  only that server's broadcasts are routed to Litty. A unicast message sent
  straight to Litty bypasses match rules, but the session bus only admits the
  same user, which is the same trust boundary as the activation socket.
- **`Notify` arguments:**

  | Argument | Value |
  |---|---|
  | app_name | `Litty` |
  | replaces_id | 0 |
  | app_icon | empty |
  | summary | title |
  | body | body (same as `notify-send` today) |
  | actions | `["default", "Open"]` |
  | hints | `{"desktop-entry": <"litty">}` (from `resources/linux/litty.desktop`) |
  | expire_timeout | -1 |
- **Tracking limit:** at most 32 notifications are tracked waiting for their
  `Notify` reply, and 32 while shown. The oldest is forgotten first.
- **Frames without a token:** a Linux client with no token to forward
  (no launcher token, or a notification click routed in-process) sends a
  plain `LTA1` frame; `LTA2` is used only when a token exists.
- **External activation tokens:**
  - stay usable for 10 s;
  - are one-shot;
  - are validated with `valid_token` (non-empty, ≤ 4096 bytes, no NUL).
- **Bus failures:** every failure to reach or use the notification bus is
  reported as `ErrorKind::Unsupported`, so `notify_pane` falls back to
  `notify-send`. A NUL in the title or body is `InvalidInput`, which is logged
  without a fallback, as on macOS.
- **X11 backend:** tokens are ignored and focus uses `_NET_ACTIVE_WINDOW` as
  today. An offered token simply expires.

## Risks & Rollback

- **Focus-stealing prevention may still refuse focus** (GNOME or KWin
  policy). The compositor decides. Verify this live and report the result.
- **XWayland (the default backend under a Wayland session)** cannot use an
  `xdg_activation` token. Focus there depends on mutter's `_NET_ACTIVE_WINDOW`
  policy. Report what was seen.
- **`+reuse` on Linux is a behavior change.** It is opt-in only; ordinary
  launches stay independent. Before this change it failed with "instance
  reuse is macOS-only".
- **Every Linux GUI launch now starts the activation worker:** one blocked
  thread with a 128 KiB stack and one socket. Benchmark modes skip it, as on
  macOS. Check with `+bench-ram` and idle CPU in Task 5.
- **First-notification latency.** Opening the bus waits for the bus daemon's
  `Hello` reply on the main thread. The settings portal already pays the same
  cost at startup.
- **libdbus contract.** Titles and bodies are always UTF-8 `String`s, and an
  interior NUL is rejected before any libdbus call.
- **macOS cannot be compiled on this host** (only the gnu target is
  installed). The shared gates in `app/mod.rs` become
  `cfg!(any(target_os = "macos", target_os = "linux"))`, which is unchanged
  on macOS. macOS CI verifies this.
- **`src/app/mod.rs:86–91`** loses its `#[cfg(target_os = "macos")]` /
  `#[cfg(target_os = "linux")]` arms in favour of one `eprintln!`. The macOS
  output is identical; Linux now also prints why the service is unavailable.
- **Rollback:** each task is its own commit; revert it. No persisted data is
  involved; the socket registry lives in the runtime dir.

## Facts found during exploration

- `src/platform/linux/notifications.rs` and `linux/activation.rs` are stubs.
  - `notify_pane` calls `process::show_notification` (`notify-send`).
  - `start_activation` returns `Unsupported`.
  - `reuse_window` returns `Err`, so `litty +reuse` fails on Linux.
- The macOS reference implementation is `src/platform/macos/activation.rs`:
  - server, client and registry: lines 1–625;
  - `NotificationRouter`, `route_queued`, `notify_pane`: lines 627–695;
  - tests: lines 697–1041.

  All of it is portable except `getpeereid`, `arc4random_buf`, `sun_len` and
  the default root path.
- The app already handles `ActivationAction::Pane` and `NewWindow`
  (`src/app/activation.rs`). Both end in `NativeWindow::focus_window`, which
  calls `WaylandWindow::focus` → `activate(true)`.
- App gates:
  - `src/app/mod.rs:75`: `start_activation_service`. The Linux error print is
    suppressed at :86–91.
  - `src/app/mod.rs:611`: the notification router.
- `handle_terminal_requests` calls `platform::notify_pane` on the main thread
  (`src/app/terminal_io.rs:65`) with the origin from `src/app/mod.rs:433`.
  The origin is `None` without a server, which keeps the plain `notify-send`
  path.
- No Linux unit test runs `run_native`, so `EPOLL_FD` is `None` in tests.
- `main.rs:51` runs `platform::capture_startup_activation()` before `+reuse`
  is dispatched. The macOS twin also calls `activation::capture_start_time()`
  (`macos/app.rs:353`); the Linux twin does not yet.
- libc 0.2.191 has `getrandom`, `ucred` and `SO_PEERCRED` for Linux gnu and
  musl.
- Docs that describe `+reuse` or notifications as macOS-only:
  - `README.md:23`
  - `docs/guide/updates-and-cli.md:18–31`
  - `docs/guide/notifications.md:50–57`
  - `ROADMAP.md:121`
  - `docs/plans/wayland-backend.md:231`
  - `docs/plans/0.14.0.md:140`
- Host: GNOME Shell 50.1 on Wayland, which sends `ActivationToken`; Intel UHD
  620; one monitor. The default Linux backend is X11 (XWayland here).
  `LITTY_BACKEND=wayland` selects native Wayland.

## File map

| File | Task | Change |
|---|---|---|
| `src/platform/linux/native/dbus.rs` | 1, 4 | new: shared libdbus wrapper |
| `src/platform/linux/native/portal.rs` | 1 | uses `dbus.rs` |
| `src/platform/linux/native/mod.rs` | 1, 4 | register `dbus`, `notify_bus` |
| `src/platform/linux/activation.rs` | 2, 3, 4 | port of the macOS server/client/router |
| `src/platform/linux/app.rs` | 2 | `capture_start_time` |
| `src/app/mod.rs` | 2, 4 | gates :75 and :611 |
| `src/platform/linux/native/wayland_activation.rs` | 3 | external token slot |
| `src/platform/linux/native/wayland.rs` | 3 | `activate(true)` uses it |
| `src/platform/linux/activation_wire.rs` | 3 | new: `LTA2` frame |
| `src/platform/linux/mod.rs` | 3 | register `activation_wire` |
| `src/platform/linux/native/notify_bus.rs` | 4 | new: D-Bus notifications |
| `src/platform/linux/native/app.rs` | 4 | epoll tag 7 |
| `src/platform/linux/notifications.rs` | 4 | real handler, `show_pane_notification` |
| docs (see Task 5) | 5 | Linux behavior |

---

### Task 1: Move the libdbus wrapper into `native/dbus.rs` (refactor, no behavior change)

**Files:**
- Create: `src/platform/linux/native/dbus.rs`
- Modify: `src/platform/linux/native/portal.rs`. Remove lines 32–61, 63–155,
  157–198, 282–305 and the test at 555–566, and rewrite `Portal::open` at
  323–360.
- Modify: `src/platform/linux/native/mod.rs:3` to add `mod dbus;` after `pub mod app;`.

**Interfaces:**
- Produces (all `pub(super)`, so visible in `native::*`):
  - constants `DBUS_TYPE_STRING`, `DBUS_TYPE_UINT32`, `DBUS_TYPE_VARIANT`,
    `DBUS_MESSAGE_TYPE_METHOD_RETURN`, `DBUS_MESSAGE_TYPE_ERROR`;
  - types `DBusMessageIter` (with `new()`), `Conn`, `Msg`, and `DBusLib`
    (every function-pointer field is `pub(super)`);
  - `DBusLib::load() -> Option<DBusLib>`;
  - `unsafe fn DBusLib::uint_in_variants(&self, &mut DBusMessageIter) -> Option<u32>`;
  - `unsafe fn DBusLib::string_at<'a>(&self, &mut DBusMessageIter) -> Option<&'a CStr>`;
  - `fn session_address() -> Option<CString>`;
  - `fn open_session(lib: &DBusLib, address: &CStr) -> Option<(*mut Conn, RawFd)>`.
- Consumes: `super::sym`.

- [x] **Step 1: Write the failing tests.** Create `dbus.rs` with only its test
  module (it fails to compile because the items do not exist yet):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bus_address_never_needs_autolaunch() {
        let os = |s: &str| Some(std::ffi::OsString::from(s));
        let yes = |_: &std::path::Path| true;
        let no = |_: &std::path::Path| false;
        assert_eq!(session_bus_address(os("unix:path=/x"), os("/run/user/1"), no).as_deref(), Some(c"unix:path=/x"));
        assert_eq!(session_bus_address(None, os("/run/user/1"), yes).as_deref(), Some(c"unix:path=/run/user/1/bus"));
        assert_eq!(session_bus_address(os(""), os("/run/user/1"), yes).as_deref(), Some(c"unix:path=/run/user/1/bus"));
        // No address and no user bus socket: no bus rather than autolaunch.
        assert_eq!(session_bus_address(None, os("/run/user/1"), no), None);
        assert_eq!(session_bus_address(None, None, yes), None);
    }

    /// An address nothing listens on is no connection, not a hang or abort.
    #[test]
    fn an_unreachable_bus_opens_nothing() {
        let Some(lib) = DBusLib::load() else {
            eprintln!("libdbus-1 not installed; skipping");
            return;
        };
        assert!(open_session(&lib, c"unix:path=/nonexistent/litty-test-bus").is_none());
    }

    /// A reachable session bus gives a registered connection with a socket.
    #[test]
    fn the_session_bus_opens_with_a_socket() {
        let (Some(lib), Some(address)) = (DBusLib::load(), session_address()) else {
            eprintln!("no libdbus-1 or session bus; skipping");
            return;
        };
        let (conn, fd) = open_session(&lib, &address).expect("session bus reachable");
        assert!(fd >= 0);
        unsafe {
            (lib.dbus_connection_close)(conn);
            (lib.dbus_connection_unref)(conn);
        }
    }
}
```

  Add `mod dbus;` after `pub mod app;` in `native/mod.rs`.

- [x] **Step 2: Run, expect FAIL.**
  `cargo test --bin litty platform::linux::native::dbus`. It fails to compile:
  `session_bus_address`, `DBusLib`, `open_session` and `session_address` are
  not found in this scope.

- [x] **Step 3: Implement.** Put this above the test module in `dbus.rs`:

```rust
//! The session bus through a dynamically loaded `libdbus-1.so.3`, shared by
//! the settings portal (`portal.rs`) and desktop notifications. Nothing links
//! against libdbus: without it, the portal keeps the dark default.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::os::fd::RawFd;

use super::sym;

pub(super) const DBUS_TYPE_STRING: c_int = b's' as c_int;
pub(super) const DBUS_TYPE_UINT32: c_int = b'u' as c_int;
pub(super) const DBUS_TYPE_VARIANT: c_int = b'v' as c_int;
pub(super) const DBUS_MESSAGE_TYPE_METHOD_RETURN: c_int = 2;
pub(super) const DBUS_MESSAGE_TYPE_ERROR: c_int = 3;
/// Portals nest the value in up to two variants (`Read` vs `ReadOne`).
const MAX_VARIANT_DEPTH: usize = 4;

/// `DBusError`: two strings, a word of flag bits, one pointer of padding.
#[repr(C)]
pub(super) struct DBusError {
    name: *const c_char,
    message: *const c_char,
    bits: u32,
    padding: *mut c_void,
}

impl DBusError {
    fn new() -> Self {
        Self { name: std::ptr::null(), message: std::ptr::null(), bits: 0, padding: std::ptr::null_mut() }
    }
}

/// `DBusMessageIter` is 72 bytes on 64-bit targets; libdbus only needs
/// stack space at least that large and pointer-aligned.
#[repr(C, align(8))]
pub(super) struct DBusMessageIter([u8; 128]);

impl DBusMessageIter {
    pub(super) fn new() -> Self {
        Self([0; 128])
    }
}

pub(super) type Conn = c_void;
pub(super) type Msg = c_void;
```

  Then move the `DBusLib` struct (portal.rs 63–104) here unchanged, except
  that each field except `_handle` gets `pub(super)` and the struct itself
  becomes `pub(super) struct DBusLib`. Keep the `#[cfg(test)]` fields as they
  are. Add an `impl DBusLib` block holding `load` (portal.rs 107–155,
  unchanged, made `pub(super) fn load`) and `uint_in_variants` / `string_at`
  (portal.rs 157–198, unchanged except `pub(super) unsafe fn`). Move
  `session_bus_address` (portal.rs 282–305) here unchanged, as a private
  `fn`. Then add:

```rust
/// The session bus address from this process's environment.
pub(super) fn session_address() -> Option<CString> {
    session_bus_address(
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS"),
        std::env::var_os("XDG_RUNTIME_DIR"),
        |p| p.exists(),
    )
}

/// A private connection to the session bus at `address`, registered on the
/// bus, and its socket; `None` when the bus cannot be reached.
pub(super) fn open_session(lib: &DBusLib, address: &CStr) -> Option<(*mut Conn, RawFd)> {
    unsafe {
        let mut err = DBusError::new();
        (lib.dbus_error_init)(&mut err);
        let conn = (lib.dbus_connection_open_private)(address.as_ptr(), &mut err);
        if conn.is_null() {
            (lib.dbus_error_free)(&mut err);
            return None;
        }
        // libdbus calls `_exit` when a bus connection drops unless told not to.
        (lib.dbus_connection_set_exit_on_disconnect)(conn, 0);
        if (lib.dbus_bus_register)(conn, &mut err) == 0 {
            (lib.dbus_error_free)(&mut err);
            (lib.dbus_connection_close)(conn);
            (lib.dbus_connection_unref)(conn);
            return None;
        }
        let mut fd: c_int = -1;
        if (lib.dbus_connection_get_unix_fd)(conn, &mut fd) == 0 {
            (lib.dbus_connection_close)(conn);
            (lib.dbus_connection_unref)(conn);
            return None;
        }
        Some((conn, fd))
    }
}
```

  In `portal.rs`:
  - Delete the moved items and the moved test (555–566).
  - Replace the imports at lines 6–12 with:

```rust
use std::ffi::{c_char, c_int, CStr};
use std::os::fd::RawFd;

use parking_lot::Mutex;

use crate::platform::gui::Theme;
use super::dbus::{
    self, Conn, DBusLib, DBusMessageIter, Msg, DBUS_MESSAGE_TYPE_ERROR, DBUS_MESSAGE_TYPE_METHOD_RETURN,
    DBUS_TYPE_STRING,
};
```

  - Keep `scheme_in_reply`, `scheme_in_signal` and `classify` in an
    `impl DBusLib { ... }` block in `portal.rs` (an inherent impl may live in
    any module of the crate).
  - Replace the body of `Portal::open` with:

```rust
    fn open() -> Option<Self> {
        let address = dbus::session_address()?;
        let lib = DBusLib::load()?;
        let (conn, fd) = dbus::open_session(&lib, &address)?;
        // Subscribe before reading, so a switch in between is not lost.
        // No error out-parameter: the match is sent without waiting for the reply.
        unsafe { (lib.dbus_bus_add_match)(conn, CHANGE_MATCH.as_ptr(), std::ptr::null_mut()) };
        let mut portal = Self { lib, conn, fd, theme: None, pending: None, opened_at: std::time::Instant::now() };
        // Not waited for here: the answer comes while the app starts up.
        portal.send_read(0);
        Some(portal)
    }
```

  - In `portal.rs` `mod tests`, add below `use super::*;`:

```rust
    use super::super::dbus::{DBUS_TYPE_UINT32, DBUS_TYPE_VARIANT};
```

- [x] **Step 4: Run, expect PASS.**
  - `cargo test --bin litty platform::linux::native`: all portal tests still pass,
    plus the three dbus tests (the bus test skips without a session bus).
  - `cargo clippy --all-targets -- -D warnings`: clean (no unused import or
    dead field).
  - Then the full per-commit gate, quoting each result: `cargo check`,
    `cargo check --target x86_64-unknown-linux-gnu`, `cargo test`,
    `cargo clippy --all-targets -- -D warnings`, `git diff --check`.

- [x] **Step 5: Commit checkpoint.** Ready to commit:
  `src/platform/linux/native/dbus.rs src/platform/linux/native/portal.rs src/platform/linux/native/mod.rs`
  — "refactor(linux): move the libdbus wrapper into its own module".

---

### Task 2: Linux activation server and client (`LTA1`), enabling `litty +reuse`

**Files:**
- Replace: `src/platform/linux/activation.rs` (the whole stub)
- Modify: `src/platform/linux/app.rs:47-48`
- Modify: `src/app/mod.rs:75` and `:86-91`

**Interfaces:**
- Produces the same public API as macOS: `ActivationServer` (`origin`,
  `accepts`), `start_activation`, `reuse_window`, plus
  `pub(super) fn capture_start_time()`.
- Keeps the stub `NotificationRouter`, `start_notification_router` and
  `notify_pane` until Task 4.
- Consumes `crate::platform::activation::{self as wire, ...}` unchanged.

- [x] **Step 1: Write the failing tests.** Copy
  `src/platform/macos/activation.rs` lines 697–1041 (the test module) to the
  end of the Linux stub, **minus** the test
  `stopping_router_drops_queued_clicks_instead_of_sending` (lines 980–996,
  which belongs to Task 4). In the copied tests:
  - `Fixture::new` uses `hex(&nonce().unwrap())`.
  - `registry_override_must_be_absolute_and_private` calls
    `root_at(Some(..), None, false)` (three call sites).

  Add these tests inside the module:

```rust
    #[test]
    fn the_default_registry_is_in_the_runtime_dir() {
        assert_eq!(default_root(Some("/run/user/1000".into())), PathBuf::from("/run/user/1000/litty-a1"));
        let fallback = PathBuf::from(format!("/tmp/litty-a1-{}", uid()));
        assert_eq!(default_root(Some("relative".into())), fallback);
        assert_eq!(default_root(Some("".into())), fallback);
        assert_eq!(default_root(None), fallback);
    }
    #[test]
    fn the_runtime_dir_registry_is_created_private() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        let made = root_at(None, Some(f.0.clone().into_os_string()), true).unwrap();
        assert_eq!(made, f.0.join("litty-a1"));
        assert_eq!(fs::metadata(&made).unwrap().permissions().mode() & 0o777, 0o700);
        fs::remove_dir(made).unwrap();
    }
```

- [x] **Step 2: Run, expect FAIL.** `cargo test --bin litty platform::linux::activation`
  fails to compile: `nonce`, `start_with_origin`, `reuse_at`, `default_root`
  and the other ported helpers are missing from the stub.

- [x] **Step 3: Implement.** Replace the stub with macOS lines 1–625, then
  append the stub's old lines 26–32 (`NotificationRouter`,
  `start_notification_router`, `notify_pane`), then the test module from
  Step 1. Apply these edits to the copied lines:

  1. Line 1 doc becomes:
     `//! Private same-user activation sockets for \`+reuse\` on Linux; one blocking worker, asleep while idle.`
  2. `nonce` (macOS 50–55) becomes:

```rust
fn nonce() -> io::Result<[u8; 16]> {
    let mut bytes = [0; 16];
    let mut filled = 0;
    while filled < bytes.len() {
        let rest = &mut bytes[filled..];
        // SAFETY: getrandom writes at most `rest.len()` bytes into `rest`.
        let read = unsafe { libc::getrandom(rest.as_mut_ptr().cast(), rest.len(), 0) };
        if read < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        filled += read as usize;
    }
    Ok(bytes)
}
```

  3. In `Origin::new`, `hex(&nonce())` becomes `hex(&nonce()?)`.
  4. `root` / `root_at` (macOS 140–156) become:

```rust
fn root(create: bool) -> io::Result<PathBuf> {
    root_at(std::env::var_os(ROOT_ENV), std::env::var_os("XDG_RUNTIME_DIR"), create)
}
/// `$XDG_RUNTIME_DIR/litty-a1` (per user, 0700, cleared at logout), else a
/// fixed per-user name in /tmp.
fn default_root(runtime: Option<std::ffi::OsString>) -> PathBuf {
    match runtime.filter(|dir| Path::new(dir).is_absolute()) {
        Some(dir) => Path::new(&dir).join("litty-a1"),
        None => PathBuf::from(format!("/tmp/litty-a1-{}", uid())),
    }
}
fn root_at(
    dir: Option<std::ffi::OsString>,
    runtime: Option<std::ffi::OsString>,
    create: bool,
) -> io::Result<PathBuf> {
    let path = match dir {
        Some(dir) if Path::new(&dir).is_absolute() => PathBuf::from(dir),
        Some(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "activation directory must be absolute",
            ));
        }
        None => default_root(runtime),
    };
    private_dir(&path, create)?;
    Ok(path)
}
```

  5. `peer` (macOS 157–170) becomes:

```rust
fn peer(stream: &UnixStream) -> io::Result<()> {
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: SO_PEERCRED writes at most `len` bytes of one ucred for the connected socket.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    if len as usize != std::mem::size_of::<libc::ucred>() || cred.uid != uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "activation peer owner mismatch",
        ));
    }
    Ok(())
}
```

  6. In `connect`, delete the `address.sun_len = ...` line (macOS 271).
  7. In `send`, `id: nonce(),` becomes `id: nonce()?,`.

  `src/platform/linux/app.rs` lines 47–48 become:

```rust
/// Capture launcher activation and the start time that names this
/// instance's activation socket, before the app starts worker threads.
pub fn capture_startup_activation() {
    super::activation::capture_start_time();
    super::native::wayland_activation::capture_startup_activation();
}
```

  `src/app/mod.rs`:
  - Line 75: `if cfg!(target_os = "macos") && self.activation.is_none()`
    becomes `if cfg!(any(target_os = "macos", target_os = "linux")) && self.activation.is_none()`.
  - Lines 86–91:

```rust
                Err(error) => {
                    #[cfg(target_os = "macos")]
                    eprintln!("litty: activation service unavailable: {}", error.kind());
                    #[cfg(target_os = "linux")]
                    let _ = error;
                }
```

    become:

```rust
                Err(error) => eprintln!("litty: activation service unavailable: {}", error.kind()),
```

- [x] **Step 4: Run, expect PASS.**
  - `cargo test --bin litty platform::linux::activation`: the ported transport,
    registry, tombstone, replay, admission and new root tests all pass.
  - `cargo test`: full suite passes.
  - `cargo clippy --all-targets -- -D warnings`: clean.
  - Then the full per-commit gate, quoting each result: `cargo check`,
    `cargo check --target x86_64-unknown-linux-gnu`, `cargo test`,
    `cargo clippy --all-targets -- -D warnings`, `git diff --check`.

- [x] **Step 5: Commit checkpoint.** Ready to commit:
  `src/platform/linux/activation.rs src/platform/linux/app.rs src/app/mod.rs`
  — "feat(linux): reuse a running instance with litty +reuse".

---

### Task 3: Forward the XDG activation token (`LTA2`)

**Files:**
- Modify: `src/platform/linux/native/wayland_activation.rs` (lines 14–27, tests 84–95)
- Modify: `src/platform/linux/native/wayland.rs:768-776` (`activate`)
- Create: `src/platform/linux/activation_wire.rs`
- Modify: `src/platform/linux/mod.rs:5` to add `mod activation_wire;` after `mod activation;`
- Modify: `src/platform/linux/activation.rs` (`send`, `serve_one`, `reuse_at`, `reuse_window`, `reuse_from`, tests)

**Interfaces:**
- `wayland_activation.rs`:
  - `pub(crate) fn valid_token(&str) -> bool` (was private);
  - `pub(crate) fn offer_activation_token(token: &str) -> bool`;
  - `pub(crate) fn take_external_token() -> Option<String>` (crate-visible so
    the activation tests can read the slot);
  - `pub(crate) fn startup_token_peek() -> Option<String>`;
  - `#[cfg(test)] pub(crate) static TOKEN_TESTS: Mutex<()>`.
- `activation_wire.rs`:
  - `pub(super) fn encode(request: &ActivationRequest, token: Option<&str>) -> io::Result<Vec<u8>>`;
  - `pub(super) fn decode(bytes: &[u8]) -> io::Result<(ActivationRequest, Option<String>)>`.
- `activation.rs`:
  - `send(root, origin, action, token: Option<&str>)`;
  - `reuse_at(root, cwd, token: Option<&str>)`;
  - `reuse_from(dir, cwd, token: Option<&str>)`.

- [x] **Step 1: Write the failing tests.**

  In `wayland_activation.rs` `mod tests` (change it to
  `pub(crate) mod tests`), add:

```rust
    /// Tests that touch the process-wide handed-over token or the notification
    /// handler run one at a time.
    pub(crate) static TOKEN_TESTS: Mutex<()> = Mutex::new(());

    #[test]
    fn handed_over_tokens_are_one_shot_and_expire() {
        let start = Instant::now();
        let mut slot = Some(("t".to_owned(), start));
        assert_eq!(take_fresh(&mut slot, start + EXTERNAL_TOKEN_TTL).as_deref(), Some("t"));
        assert_eq!(take_fresh(&mut slot, start), None, "a token is used once");
        let mut stale = Some(("t".to_owned(), start));
        assert_eq!(take_fresh(&mut stale, start + EXTERNAL_TOKEN_TTL + Duration::from_millis(1)), None);
        assert!(stale.is_none(), "a stale token is dropped, not kept");
    }

    #[test]
    fn only_valid_tokens_are_offered() {
        let _serial = TOKEN_TESTS.lock();
        take_external_token();
        assert!(!offer_activation_token(""));
        assert!(!offer_activation_token("a\0b"));
        assert!(!offer_activation_token(&"a".repeat(4097)));
        assert_eq!(take_external_token(), None);
        assert!(offer_activation_token("fresh"));
        assert_eq!(take_external_token().as_deref(), Some("fresh"));
        assert_eq!(take_external_token(), None);
    }
```

  Create `activation_wire.rs` with only its tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::activation::ActivationAction;

    fn request(action: ActivationAction) -> ActivationRequest {
        ActivationRequest { id: [1; 16], session: [2; 16], expires_ns: 200, action }
    }

    #[test]
    fn frames_round_trip_with_and_without_a_token() {
        for action in [ActivationAction::NewWindow("/tmp/tiếng việt".into()), ActivationAction::Pane(42)] {
            let r = request(action);
            assert_eq!(encode(&r, None).unwrap(), r.encode().unwrap(), "no token is a plain LTA1 frame");
            assert_eq!(decode(&r.encode().unwrap()).unwrap(), (r.clone(), None));
            let framed = encode(&r, Some("tok:en/1")).unwrap();
            assert_eq!(&framed[..4], b"LTA2");
            assert_eq!(decode(&framed).unwrap(), (r, Some("tok:en/1".to_owned())));
        }
    }

    #[test]
    fn malformed_token_frames_are_rejected() {
        let r = request(ActivationAction::Pane(42));
        let framed = encode(&r, Some("token")).unwrap();
        for cut in 0..framed.len() {
            assert!(decode(&framed[..cut]).is_err(), "cut at {cut}");
        }
        let mut extra = framed.clone();
        extra.push(0);
        assert!(decode(&extra).is_err());
        for token in ["", "a\0b", &"a".repeat(4097)] {
            assert!(encode(&r, Some(token)).is_err());
        }
        let inner = r.encode().unwrap();
        let frame = |token: &[u8], inner: &[u8]| {
            let mut out = b"LTA2".to_vec();
            out.extend_from_slice(&(token.len() as u16).to_be_bytes());
            out.extend_from_slice(token);
            out.extend_from_slice(inner);
            out
        };
        assert!(decode(&frame(b"", &inner)).is_err(), "empty token");
        assert!(decode(&frame(b"\xff\xfe", &inner)).is_err(), "token not UTF-8");
        assert!(decode(&frame(b"a\0", &inner)).is_err(), "token with NUL");
        assert!(decode(&frame(b"t", &framed)).is_err(), "nested LTA2");
    }
}
```

  In `linux/activation.rs` tests:
  - Update every existing call: `reuse_at(&f.0, X)` → `reuse_at(&f.0, X, None)`,
    `send(&fixture.0, &..., ActivationAction::Pane(42))` → add `, None`, and
    `reuse_from(x, "/tmp")` → `reuse_from(x, "/tmp", None)`.
  - Add:

```rust
    #[test]
    fn reuse_forwards_its_token_once_and_replays_never_reoffer() {
        use crate::platform::linux::native::wayland_activation::{tests::TOKEN_TESTS, take_external_token};
        let _serial = TOKEN_TESTS.lock();
        take_external_token();
        let f = Fixture::new();
        let (sent, accepted) = mpsc::sync_channel(4);
        let server = start_at(
            f.0.clone(),
            Arc::new(move |request, reply| {
                sent.send(request.action).unwrap();
                reply.send(Ok(())).unwrap();
            }),
        )
        .unwrap();
        assert!(reuse_at(&f.0, "/tmp", Some("launcher-token")).unwrap());
        assert_eq!(accepted.recv_timeout(TIMEOUT).unwrap(), ActivationAction::NewWindow("/tmp".into()));
        assert_eq!(take_external_token().as_deref(), Some("launcher-token"));

        let request = ActivationRequest {
            id: [7; 16],
            session: server.origin.session,
            expires_ns: monotonic_ns().unwrap() + TIMEOUT.as_nanos() as u64,
            action: ActivationAction::Pane(42),
        };
        let exchange = || {
            let mut connection = connect(&f.0.join(&server.origin.name).join("s")).unwrap();
            wire::write_frame(&mut connection, &activation_wire::encode(&request, Some("click-token")).unwrap()).unwrap();
            wire::read_frame(&mut connection).unwrap()
        };
        assert_eq!(exchange(), [0]);
        assert_eq!(take_external_token().as_deref(), Some("click-token"));
        assert_eq!(exchange(), [0], "a replay gets the recorded answer");
        assert_eq!(take_external_token(), None, "a replay must not offer its token again");
        drop(server);
    }

    #[test]
    fn an_invalid_token_is_an_error_and_reaches_no_app() {
        let f = Fixture::new();
        let (sent, accepted) = mpsc::sync_channel(4);
        let server = start_at(
            f.0.clone(),
            Arc::new(move |request, reply| {
                sent.send(request.action).unwrap();
                reply.send(Ok(())).unwrap();
            }),
        )
        .unwrap();
        assert_eq!(reuse_at(&f.0, "/tmp", Some("")).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert!(accepted.try_recv().is_err());
        drop(server);
    }
```

  The ported `real_requests_reject_stale_sessions_and_do_not_replay_actions`
  still sends plain `request.encode()` (`LTA1`). It is the regression test
  that the server keeps accepting `LTA1` clients.

- [x] **Step 2: Run, expect FAIL.** `cargo test --bin litty platform::linux` fails to
  compile:
  - `take_fresh`, `EXTERNAL_TOKEN_TTL`, `offer_activation_token`,
    `take_external_token`, `encode` and `decode` are not found;
  - `reuse_at` / `send` / `reuse_from` take the wrong number of arguments.

- [x] **Step 3: Implement.**

  `wayland_activation.rs`. Replace lines 26–27 (`startup_token` and
  `valid_token`) with:

```rust
pub(super) fn startup_token() -> Option<String> { STARTUP.get()?.lock().take() }
/// The launcher token without using it up: `+reuse` hands it to the running
/// instance, yet keeps it for its own first window if none answers.
pub(crate) fn startup_token_peek() -> Option<String> { STARTUP.get()?.lock().clone() }
pub(crate) fn valid_token(token: &str) -> bool { !token.is_empty() && token.len() <= 4096 && !token.contains('\0') }

/// How long a token handed over by another process or the notification
/// server stays usable: compositors expire unused tokens within seconds.
const EXTERNAL_TOKEN_TTL: Duration = Duration::from_secs(10);
/// A token handed over by another process or the notification server, and when.
static EXTERNAL: Mutex<Option<(String, Instant)>> = Mutex::new(None);

/// Keep `token` for the next explicit focus; `false` (and nothing kept) when
/// it is not a usable token.
pub(crate) fn offer_activation_token(token: &str) -> bool {
    if !valid_token(token) {
        return false;
    }
    *EXTERNAL.lock() = Some((token.to_owned(), Instant::now()));
    true
}
/// The handed-over token, once, while it is still fresh.
pub(crate) fn take_external_token() -> Option<String> { take_fresh(&mut EXTERNAL.lock(), Instant::now()) }
fn take_fresh(slot: &mut Option<(String, Instant)>, now: Instant) -> Option<String> {
    let (token, at) = slot.take()?;
    (now.saturating_duration_since(at) <= EXTERNAL_TOKEN_TTL).then_some(token)
}
```

  `wayland.rs`. Replace `activate` (lines 768–776) with:

```rust
    fn activate(&self, focus: bool) {
        let guard = self.0.client.queue.lock();
        let state = &guard.1;
        let Some(manager) = &state.activation_manager else { return; };
        if focus && let Some(token) = super::wayland_activation::take_external_token() {
            // A token from `+reuse` or a notification click already carries the
            // user's intent; a new request would lack it and only highlight.
            self.0.shared.state.lock().activation.cancel();
            manager.activate(token, self.0.window.wl_surface());
            super::app::wakeup_main_loop();
            return;
        }
        let grant = if focus { state.activation_input.as_ref() } else { None };
        self.0.shared.state.lock().activation.request(manager, &self.0.client.qh,
            self.0.window.wl_surface(), Arc::downgrade(&self.0.shared), grant);
        super::app::wakeup_main_loop();
    }
```

  `activation_wire.rs`. Put this above its tests:

```rust
//! Linux activation frames. `LTA2` carries an XDG activation token ahead of
//! an unchanged `LTA1` request, so a running instance can focus the window
//! it opens; plain `LTA1` frames still decode.
use crate::platform::activation::{self as wire, ActivationRequest};
use super::native::wayland_activation::valid_token;
use std::io;

const MAGIC: &[u8; 4] = b"LTA2";

/// `request` as a frame, with `token` ahead of it when there is one.
pub(super) fn encode(request: &ActivationRequest, token: Option<&str>) -> io::Result<Vec<u8>> {
    let inner = request.encode()?;
    let Some(token) = token else { return Ok(inner) };
    if !valid_token(token) {
        return Err(wire::invalid());
    }
    // `valid_token` bounds a token to 4096 bytes, so its length fits a u16.
    let mut out = Vec::with_capacity(MAGIC.len() + 2 + token.len() + inner.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(token.len() as u16).to_be_bytes());
    out.extend_from_slice(token.as_bytes());
    out.extend_from_slice(&inner);
    Ok(out)
}

/// The request in an `LTA1` or `LTA2` frame, and the `LTA2` token.
pub(super) fn decode(bytes: &[u8]) -> io::Result<(ActivationRequest, Option<String>)> {
    let Some(rest) = bytes.strip_prefix(MAGIC) else {
        return Ok((ActivationRequest::decode(bytes)?, None));
    };
    let (len, rest) = rest.split_first_chunk::<2>().ok_or_else(wire::invalid)?;
    let len = usize::from(u16::from_be_bytes(*len));
    if rest.len() < len {
        return Err(wire::invalid());
    }
    let (token, inner) = rest.split_at(len);
    let token = std::str::from_utf8(token).map_err(|_| wire::invalid())?;
    if !valid_token(token) {
        return Err(wire::invalid());
    }
    // `ActivationRequest::decode` accepts only a whole `LTA1` frame: a nested
    // `LTA2` or trailing bytes are rejected there.
    Ok((ActivationRequest::decode(inner)?, Some(token.to_owned())))
}
```

  `linux/activation.rs`:
  - Imports: add `use super::activation_wire;` after the `wire` import.
  - `send`: the signature becomes
    `fn send(root: &Path, origin: &Origin, action: ActivationAction, token: Option<&str>) -> io::Result<()>`,
    and `wire::write_frame(&mut stream, &req.encode()?)` becomes
    `wire::write_frame(&mut stream, &activation_wire::encode(&req, token)?)`.
  - `serve_one`: `let req = ActivationRequest::decode(&wire::read_frame(&mut stream)?)?;`
    becomes `let (req, token) = activation_wire::decode(&wire::read_frame(&mut stream)?)?;`.
    Between the `if !admission.ready() { ... }` block and
    `let (reply, rx) = mpsc::sync_channel(1);`, insert:

```rust
        if let Some(token) = &token {
            // Before the app sees the request, so the focus it triggers can
            // use the token. A replay returned above and never gets here.
            super::native::wayland_activation::offer_activation_token(token);
        }
```

  - `reuse_at`: the signature becomes
    `fn reuse_at(root: &Path, cwd: &str, token: Option<&str>) -> io::Result<bool>`,
    and its call becomes `send(root, &origin, ActivationAction::NewWindow(cwd.into()), token)`.
  - `reuse_window`: its body becomes:

```rust
    wire::validate_cwd(cwd)?;
    let token = super::native::wayland_activation::startup_token_peek();
    reuse_from(root(false), cwd, token.as_deref())
```

  - `reuse_from`: the signature becomes
    `fn reuse_from(dir: io::Result<PathBuf>, cwd: &str, token: Option<&str>) -> io::Result<bool>`,
    and `Ok(dir) => reuse_at(&dir, cwd),` becomes `Ok(dir) => reuse_at(&dir, cwd, token),`.

- [x] **Step 4: Run, expect PASS.**
  - `cargo test --bin litty platform::linux`: the new token, wire and forwarding
    tests and every older test pass.
  - `cargo test`: full suite.
  - `cargo clippy --all-targets -- -D warnings`: clean.
  - Then the full per-commit gate, quoting each result: `cargo check`,
    `cargo check --target x86_64-unknown-linux-gnu`, `cargo test`,
    `cargo clippy --all-targets -- -D warnings`, `git diff --check`.

  `startup_token_peek` is one line over a process-wide `OnceLock` that tests
  cannot reset safely. It has no unit test; Task 5 E2E step 1 covers it.
  Report that.

- [x] **Step 5: Commit checkpoint.** Ready to commit:
  `src/platform/linux/native/wayland_activation.rs src/platform/linux/native/wayland.rs src/platform/linux/activation_wire.rs src/platform/linux/mod.rs src/platform/linux/activation.rs`
  — "feat(linux): forward the launcher activation token on reuse".

---

### Task 4: D-Bus notifications with click-through

**Files:**
- Modify: `src/platform/linux/native/dbus.rs`. Add constants and make the
  container calls non-test; add test-only `dbus_message_get_signature`.
- Create: `src/platform/linux/native/notify_bus.rs`
- Modify: `src/platform/linux/native/mod.rs` to add
  `pub(super) mod notify_bus;` after `pub mod monitor;`.
- Modify: `src/platform/linux/native/app.rs`:
  - add `watch_notification_bus` after `register_key_repeat_fd` (line 62);
  - add the effects buffer at 311–314 and tag 7 at 400–402;
  - add `notification_bus_readable` after `portal_readable`;
  - add a test.
- Replace: `src/platform/linux/notifications.rs`
- Modify: `src/platform/linux/activation.rs`. Replace the stub router and
  `notify_pane` with a port of macOS 627–695, and add the router test.
- Modify: `src/app/mod.rs:611`

**Interfaces:**
- `dbus.rs` adds `DBUS_TYPE_INT32`, `DBUS_TYPE_ARRAY`, `DBUS_TYPE_DICT_ENTRY`
  and `DBUS_MESSAGE_TYPE_SIGNAL`. The fields `dbus_message_iter_open_container`
  and `dbus_message_iter_close_container` are no longer `cfg(test)`.
- `notify_bus.rs`:
  - `pub(in crate::platform::linux) fn show(origin: &str, pane: u64, title: &str, body: &str) -> io::Result<()>`;
  - `pub(super) fn dispatch(effects: &mut Vec<Effect>) -> Result<(), Disconnected>`;
  - `pub(super) fn watch_fd() -> Option<RawFd>`;
  - `pub(super) fn close(effects: &mut Vec<Effect>)` (unanswered calls become `Fallback`);
  - `pub(super) enum Effect { Fallback { title, body }, Click { origin, pane, token } }`.
- `app.rs`: `pub fn watch_notification_bus(fd: RawFd) -> bool`.
- `notifications.rs`:
  `pub(crate) fn deliver_notification_click(origin: String, pane: u64)`.

- [x] **Step 1: Write the failing tests.** Create `notify_bus.rs` with only its
  tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_char;

    const ORIGIN: &str = "0000000000000001-1-00000000000000000000000000000001";

    fn route(pane: u64) -> Route {
        Route { origin: ORIGIN.to_owned(), pane }
    }

    #[test]
    fn a_click_selects_the_pane_with_the_servers_token() {
        let mut tracker = Tracker::default();
        tracker.sent(5, route(9), "title".into(), "body".into());
        assert_eq!(tracker.on(Incoming::Shown { serial: 5, id: 77 }), None);
        assert_eq!(tracker.on(Incoming::Token { id: 77, token: "tok".into() }), None);
        assert_eq!(
            tracker.on(Incoming::Invoked { id: 77, action: "default".into() }),
            Some(Effect::Click { origin: ORIGIN.into(), pane: 9, token: Some("tok".into()) })
        );
        assert_eq!(tracker.on(Incoming::Invoked { id: 77, action: "default".into() }), None, "one click per notification");
    }

    #[test]
    fn a_click_without_a_token_still_selects_the_pane() {
        let mut tracker = Tracker::default();
        tracker.sent(5, route(9), "t".into(), "b".into());
        tracker.on(Incoming::Shown { serial: 5, id: 77 });
        assert_eq!(
            tracker.on(Incoming::Invoked { id: 77, action: "default".into() }),
            Some(Effect::Click { origin: ORIGIN.into(), pane: 9, token: None })
        );
    }

    #[test]
    fn a_refused_notify_falls_back_to_the_helper_once() {
        let mut tracker = Tracker::default();
        tracker.sent(5, route(9), "title".into(), "body".into());
        assert_eq!(tracker.on(Incoming::Failed { serial: 5 }), Some(Effect::Fallback { title: "title".into(), body: "body".into() }));
        assert_eq!(tracker.on(Incoming::Failed { serial: 5 }), None);
    }

    #[test]
    fn unrelated_messages_change_nothing() {
        let mut tracker = Tracker::default();
        // Replies to other calls (AddMatch) and signals for other clients.
        assert_eq!(tracker.on(Incoming::Failed { serial: 1 }), None);
        assert_eq!(tracker.on(Incoming::Shown { serial: 1, id: 3 }), None);
        assert_eq!(tracker.on(Incoming::Invoked { id: 3, action: "default".into() }), None);
        assert_eq!(tracker.on(Incoming::Token { id: 3, token: "t".into() }), None);
        assert_eq!(tracker.on(Incoming::Other), None);
        tracker.sent(5, route(9), "t".into(), "b".into());
        tracker.on(Incoming::Shown { serial: 5, id: 77 });
        assert_eq!(tracker.on(Incoming::Invoked { id: 77, action: "snooze".into() }), None, "only the default action clicks");
        assert_eq!(tracker.on(Incoming::Closed { id: 77 }), None);
        assert_eq!(tracker.on(Incoming::Invoked { id: 77, action: "default".into() }), None, "closed is forgotten");
    }

    #[test]
    fn a_lost_bus_falls_back_for_unanswered_calls_only() {
        let mut tracker = Tracker::default();
        tracker.sent(5, route(9), "shown".into(), "b".into());
        tracker.on(Incoming::Shown { serial: 5, id: 77 });
        tracker.sent(6, route(9), "waiting".into(), "b".into());
        let mut effects = Vec::new();
        tracker.abandon(&mut effects);
        assert_eq!(effects, [Effect::Fallback { title: "waiting".into(), body: "b".into() }]);
        assert_eq!(tracker.on(Incoming::Invoked { id: 77, action: "default".into() }), None, "nothing tracked after");
    }

    #[test]
    fn tracking_is_bounded() {
        let mut tracker = Tracker::default();
        for serial in 0..=MAX_TRACKED as u32 {
            tracker.sent(serial, route(1), String::new(), String::new());
        }
        assert_eq!(tracker.on(Incoming::Failed { serial: 0 }), None, "the oldest unanswered call was forgotten");
        for serial in 1..=MAX_TRACKED as u32 {
            tracker.on(Incoming::Shown { serial, id: serial + 100 });
        }
        tracker.sent(99, route(2), String::new(), String::new());
        tracker.on(Incoming::Shown { serial: 99, id: 999 });
        assert_eq!(tracker.on(Incoming::Invoked { id: 101, action: "default".into() }), None, "the oldest shown one was forgotten");
        assert!(tracker.on(Incoming::Invoked { id: 999, action: "default".into() }).is_some());
    }

    #[test]
    fn without_an_event_loop_notifications_are_unsupported() {
        // No unit test runs the event loop, so the bus socket cannot be
        // watched and nothing is left open.
        *BUS.lock() = State::Unopened;
        assert_eq!(show(ORIGIN, 1, "t", "b").unwrap_err().kind(), io::ErrorKind::Unsupported);
        assert!(watch_fd().is_none());
        assert_eq!(show(ORIGIN, 1, "t\0", "b").unwrap_err().kind(), io::ErrorKind::InvalidInput);
        *BUS.lock() = State::Unopened;
    }

    fn lib() -> Option<DBusLib> {
        let lib = DBusLib::load();
        if lib.is_none() {
            eprintln!("libdbus-1 not installed; skipping");
        }
        lib
    }

    unsafe fn append_text(lib: &DBusLib, iter: &mut DBusMessageIter, s: &CStr) {
        let ptr = s.as_ptr();
        unsafe { (lib.dbus_message_iter_append_basic)(iter, DBUS_TYPE_STRING, (&ptr as *const *const c_char).cast()) };
    }

    unsafe fn signal(lib: &DBusLib, member: &CStr, id: u32, text: Option<&CStr>) -> *mut Msg {
        unsafe {
            let msg = (lib.dbus_message_new_signal)(NOTIFY_PATH.as_ptr(), NOTIFY_IFACE.as_ptr(), member.as_ptr());
            let mut iter = DBusMessageIter::new();
            (lib.dbus_message_iter_init_append)(msg, &mut iter);
            (lib.dbus_message_iter_append_basic)(&mut iter, DBUS_TYPE_UINT32, (&id as *const u32).cast());
            match text {
                Some(text) => append_text(lib, &mut iter, text),
                None => {
                    let reason = 2u32;
                    (lib.dbus_message_iter_append_basic)(&mut iter, DBUS_TYPE_UINT32, (&reason as *const u32).cast());
                }
            }
            msg
        }
    }

    /// The call matches the specification's `Notify(susssasa{sv}i)` with
    /// one default action, so servers accept it and send `ActionInvoked`.
    #[test]
    fn notify_calls_have_the_spec_signature_and_a_default_action() {
        let Some(lib) = lib() else { return };
        unsafe {
            let msg = lib.notify_call(c"title", c"body");
            assert!(!msg.is_null());
            assert_eq!(CStr::from_ptr((lib.dbus_message_get_signature)(msg)), c"susssasa{sv}i");
            let mut iter = DBusMessageIter::new();
            (lib.dbus_message_iter_init)(msg, &mut iter);
            assert_eq!(lib.string_at(&mut iter), Some(APP_NAME));
            for _ in 0..5 {
                (lib.dbus_message_iter_next)(&mut iter);
            }
            let mut actions = DBusMessageIter::new();
            (lib.dbus_message_iter_recurse)(&mut iter, &mut actions);
            assert_eq!(lib.string_at(&mut actions), Some(DEFAULT_ACTION));
            (lib.dbus_message_iter_next)(&mut actions);
            assert_eq!(lib.string_at(&mut actions), Some(c"Open"));
            (lib.dbus_message_unref)(msg);
        }
    }

    #[test]
    fn replies_and_signals_are_classified() {
        let Some(lib) = lib() else { return };
        unsafe {
            let call = lib.notify_call(c"t", c"b");
            (lib.dbus_message_set_serial)(call, 7);
            let reply = (lib.dbus_message_new_method_return)(call);
            let mut iter = DBusMessageIter::new();
            (lib.dbus_message_iter_init_append)(reply, &mut iter);
            let id = 42u32;
            (lib.dbus_message_iter_append_basic)(&mut iter, DBUS_TYPE_UINT32, (&id as *const u32).cast());
            assert_eq!(lib.notification_incoming(reply), Incoming::Shown { serial: 7, id: 42 });
            (lib.dbus_message_unref)(reply);

            let empty = (lib.dbus_message_new_method_return)(call);
            assert_eq!(lib.notification_incoming(empty), Incoming::Failed { serial: 7 });
            (lib.dbus_message_unref)(empty);

            let error = (lib.dbus_message_new_error)(call, c"org.freedesktop.DBus.Error.ServiceUnknown".as_ptr(), c"no".as_ptr());
            assert_eq!(lib.notification_incoming(error), Incoming::Failed { serial: 7 });
            (lib.dbus_message_unref)(error);
            (lib.dbus_message_unref)(call);

            for (member, text, expected) in [
                (c"ActionInvoked", Some(c"default"), Incoming::Invoked { id: 42, action: "default".into() }),
                (c"ActivationToken", Some(c"tok"), Incoming::Token { id: 42, token: "tok".into() }),
                (c"NotificationClosed", None, Incoming::Closed { id: 42 }),
            ] {
                let msg = signal(&lib, member, 42, text);
                assert_eq!(lib.notification_incoming(msg), expected, "{member:?}");
                (lib.dbus_message_unref)(msg);
            }
            let other = (lib.dbus_message_new_signal)(NOTIFY_PATH.as_ptr(), c"org.example.Other".as_ptr(), c"ActionInvoked".as_ptr());
            let mut iter = DBusMessageIter::new();
            (lib.dbus_message_iter_init_append)(other, &mut iter);
            (lib.dbus_message_iter_append_basic)(&mut iter, DBUS_TYPE_UINT32, (&id as *const u32).cast());
            append_text(&lib, &mut iter, c"default");
            assert_eq!(lib.notification_incoming(other), Incoming::Other);
            (lib.dbus_message_unref)(other);
        }
    }
}
```

  In `native/app.rs` `mod tests`, add:

```rust
    /// A click offers the server's token for the focus it triggers, then
    /// reaches the router with its pane.
    #[test]
    fn a_click_offers_its_token_and_reaches_the_router() {
        use super::super::wayland_activation::{take_external_token, tests::TOKEN_TESTS};
        let _serial = TOKEN_TESTS.lock();
        take_external_token();
        let (sent, received) = std::sync::mpsc::channel();
        crate::platform::linux::set_notification_handler(Some(std::sync::Arc::new(move |origin, pane| {
            sent.send((origin, pane)).unwrap();
        })));
        apply_notification_effect(super::super::notify_bus::Effect::Click {
            origin: "o".into(),
            pane: 3,
            token: Some("click-token".into()),
        });
        crate::platform::linux::set_notification_handler(None);
        assert_eq!(received.try_recv().unwrap(), ("o".to_owned(), 3));
        assert_eq!(take_external_token().as_deref(), Some("click-token"));
    }

    /// A bus socket epoll refuses is reported, so the bus is closed and
    /// notifications fall back instead of their replies going unread.
    #[test]
    fn notification_bus_sockets_are_watched_only_when_epoll_accepts_them() {
        assert!(!watch_bus_fd(None, 0), "no event loop");
        let epoll = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        assert!(epoll >= 0);
        assert!(!watch_bus_fd(Some(epoll), -1));
        let mut pipe = [0; 2];
        assert_eq!(unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) }, 0);
        assert!(watch_bus_fd(Some(epoll), pipe[0]));
        unsafe {
            libc::close(pipe[0]);
            libc::close(pipe[1]);
            libc::close(epoll);
        }
    }
```

  Replace the body of `linux/notifications.rs` tests (new module) with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_boundaries_reject_bad_routes_and_text_before_the_bus() {
        let valid = "0000000000000001-1-00000000000000000000000000000001";
        assert!(valid_origin(valid));
        for origin in ["../bad", "", "0-1-aa", "0000000000000001-1-🙂"] {
            assert!(!valid_origin(origin));
        }
        let rejects = |origin: &str, pane: u64, title: &str, body: &str| {
            show_pane_notification(origin, pane, title, body).unwrap_err().kind() == io::ErrorKind::InvalidInput
        };
        assert!(rejects("../bad", 1, "title", "body"));
        assert!(rejects(valid, 0, "title", "body"));
        assert!(rejects(valid, 1, "\0", "body"));
        assert!(rejects(valid, 1, "title", "a\0b"));
        assert!(rejects(valid, 1, &"x".repeat(4097), "body"));
        assert!(rejects(valid, 1, "title", &"x".repeat(16385)));
    }

    #[test]
    fn clicks_reach_the_router_only_while_one_is_registered() {
        // `HANDLER` is process-wide; the native app test also sets it.
        let _serial = super::super::native::wayland_activation::tests::TOKEN_TESTS.lock();
        let (sent, received) = std::sync::mpsc::channel();
        set_notification_handler(Some(Arc::new(move |origin, pane| sent.send((origin, pane)).unwrap())));
        deliver_notification_click("o".into(), 4);
        assert_eq!(received.try_recv().unwrap(), ("o".to_owned(), 4));
        set_notification_handler(None);
        deliver_notification_click("o".into(), 5);
        assert!(received.try_recv().is_err());
    }
}
```

  In `linux/activation.rs` tests, add back the macOS router test unchanged:
  `stopping_router_drops_queued_clicks_instead_of_sending` (macOS lines
  980–996).

- [x] **Step 2: Run, expect FAIL.** `cargo test --bin litty platform::linux` fails to
  compile:
  - `Tracker`, `Incoming`, `Effect`, `Route`, `MAX_TRACKED`, `BUS`, `State`,
    `show`, `notify_call`, `notification_incoming`, `watch_bus_fd`,
    `valid_origin`, `deliver_notification_click`, `route_queued`,
    `apply_notification_effect` and `Tracker::abandon` are not found;
  - `dbus_message_get_signature` is not a field of `DBusLib`.

- [x] **Step 3: Implement.** — deviated: `serial` hoisted out of the `unsafe` block in `Bus::send` (the plan block declared it inside, out of scope at `tracker.sent`); plan block fixed.

  **`dbus.rs`:**
  - Add after `DBUS_MESSAGE_TYPE_ERROR`:

```rust
pub(super) const DBUS_TYPE_INT32: c_int = b'i' as c_int;
pub(super) const DBUS_TYPE_ARRAY: c_int = b'a' as c_int;
pub(super) const DBUS_TYPE_DICT_ENTRY: c_int = b'e' as c_int;
pub(super) const DBUS_MESSAGE_TYPE_SIGNAL: c_int = 4;
```

  - Remove `#[cfg(test)]` from the `dbus_message_iter_open_container` and
    `dbus_message_iter_close_container` fields and from their two `sym` lines
    in `load`.
  - Add a test-only field after `dbus_message_set_serial`:

```rust
    #[cfg(test)]
    pub(super) dbus_message_get_signature: unsafe extern "C" fn(*mut Msg) -> *const c_char,
```

    and its loader line in `load`:

```rust
                #[cfg(test)]
                dbus_message_get_signature: sym(handle, c"dbus_message_get_signature")?,
```

  **`notify_bus.rs`.** Put this above its tests:

```rust
//! Desktop notifications through `org.freedesktop.Notifications` on the
//! session bus, so a click can select the pane that sent it. The bus opens
//! with the first notification; without one, `show` reports `Unsupported`
//! and the caller uses `notify-send`.

use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::io;
use std::os::fd::RawFd;

use parking_lot::Mutex;

use super::dbus::{
    self, Conn, DBusLib, DBusMessageIter, Msg, DBUS_MESSAGE_TYPE_ERROR, DBUS_MESSAGE_TYPE_METHOD_RETURN,
    DBUS_MESSAGE_TYPE_SIGNAL, DBUS_TYPE_ARRAY, DBUS_TYPE_DICT_ENTRY, DBUS_TYPE_INT32, DBUS_TYPE_STRING,
    DBUS_TYPE_UINT32, DBUS_TYPE_VARIANT,
};

const NOTIFY_DEST: &CStr = c"org.freedesktop.Notifications";
const NOTIFY_PATH: &CStr = c"/org/freedesktop/Notifications";
const NOTIFY_IFACE: &CStr = c"org.freedesktop.Notifications";
/// Only the notification server's own broadcasts are routed here.
const SIGNAL_MATCH: &CStr = c"type='signal',sender='org.freedesktop.Notifications',\
interface='org.freedesktop.Notifications',path='/org/freedesktop/Notifications'";
const APP_NAME: &CStr = c"Litty";
/// The desktop entry (`resources/linux/litty.desktop`) gives the icon and name.
const DESKTOP_ENTRY: &CStr = c"litty";
/// The action servers invoke when the notification itself is clicked.
const DEFAULT_ACTION: &CStr = c"default";
/// Notifications remembered at once, both waiting for their `Notify` reply
/// and while shown; the oldest is forgotten first, losing only its click.
const MAX_TRACKED: usize = 32;

/// The pane a notification's click selects.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Route {
    origin: String,
    pane: u64,
}

/// A `Notify` call awaiting its reply; the text is kept for the fallback.
struct Sent {
    serial: u32,
    route: Route,
    title: String,
    body: String,
}

/// A notification the server shows, with the token it sent for it.
struct Live {
    id: u32,
    route: Route,
    token: Option<String>,
}

/// A message from the bus, as far as notifications are concerned.
#[derive(Debug, PartialEq, Eq)]
enum Incoming {
    /// `Notify` answered with the notification's id.
    Shown { serial: u32, id: u32 },
    /// `Notify` answered with an error (or no id).
    Failed { serial: u32 },
    /// `ActivationToken`: sent just before `ActionInvoked`.
    Token { id: u32, token: String },
    Invoked { id: u32, action: String },
    Closed { id: u32 },
    Other,
}

/// What the event loop does after reading the bus.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Effect {
    /// The server refused the notification: show it with `notify-send`.
    Fallback { title: String, body: String },
    /// The user clicked it: select `pane`, focusing with `token`.
    Click { origin: String, pane: u64, token: Option<String> },
}

/// Which notifications this process sent, and what their events mean.
#[derive(Default)]
struct Tracker {
    sent: VecDeque<Sent>,
    live: VecDeque<Live>,
}

impl Tracker {
    fn sent(&mut self, serial: u32, route: Route, title: String, body: String) {
        if self.sent.len() == MAX_TRACKED {
            self.sent.pop_front();
        }
        self.sent.push_back(Sent { serial, route, title, body });
    }

    /// The bus went away: every call it never answered falls back.
    fn abandon(&mut self, effects: &mut Vec<Effect>) {
        effects.extend(self.sent.drain(..).map(|sent| Effect::Fallback { title: sent.title, body: sent.body }));
        self.live.clear();
    }

    fn take_sent(&mut self, serial: u32) -> Option<Sent> {
        let at = self.sent.iter().position(|s| s.serial == serial)?;
        self.sent.remove(at)
    }

    fn take_live(&mut self, id: u32) -> Option<Live> {
        let at = self.live.iter().position(|l| l.id == id)?;
        self.live.remove(at)
    }

    fn on(&mut self, incoming: Incoming) -> Option<Effect> {
        match incoming {
            Incoming::Shown { serial, id } => {
                let sent = self.take_sent(serial)?;
                if self.live.len() == MAX_TRACKED {
                    self.live.pop_front();
                }
                self.live.push_back(Live { id, route: sent.route, token: None });
                None
            }
            Incoming::Failed { serial } => {
                let sent = self.take_sent(serial)?;
                Some(Effect::Fallback { title: sent.title, body: sent.body })
            }
            Incoming::Token { id, token } => {
                if let Some(live) = self.live.iter_mut().find(|l| l.id == id) {
                    live.token = Some(token);
                }
                None
            }
            Incoming::Invoked { id, action } => {
                if action.as_bytes() != DEFAULT_ACTION.to_bytes() {
                    return None;
                }
                let live = self.take_live(id)?;
                Some(Effect::Click { origin: live.route.origin, pane: live.route.pane, token: live.token })
            }
            Incoming::Closed { id } => {
                self.take_live(id);
                None
            }
            Incoming::Other => None,
        }
    }
}

impl DBusLib {
    unsafe fn append_str(&self, iter: &mut DBusMessageIter, s: &CStr) -> bool {
        let ptr = s.as_ptr();
        unsafe { (self.dbus_message_iter_append_basic)(iter, DBUS_TYPE_STRING, (&ptr as *const *const std::ffi::c_char).cast()) != 0 }
    }

    unsafe fn append_u32(&self, iter: &mut DBusMessageIter, value: u32) -> bool {
        unsafe { (self.dbus_message_iter_append_basic)(iter, DBUS_TYPE_UINT32, (&value as *const u32).cast()) != 0 }
    }

    unsafe fn append_i32(&self, iter: &mut DBusMessageIter, value: i32) -> bool {
        unsafe { (self.dbus_message_iter_append_basic)(iter, DBUS_TYPE_INT32, (&value as *const i32).cast()) != 0 }
    }

    /// `as`: the default action, labelled for servers that show buttons.
    unsafe fn append_actions(&self, args: &mut DBusMessageIter) -> bool {
        let mut actions = DBusMessageIter::new();
        unsafe {
            (self.dbus_message_iter_open_container)(args, DBUS_TYPE_ARRAY, c"s".as_ptr(), &mut actions) != 0
                && self.append_str(&mut actions, DEFAULT_ACTION)
                && self.append_str(&mut actions, c"Open")
                && (self.dbus_message_iter_close_container)(args, &mut actions) != 0
        }
    }

    /// `a{sv}`: `desktop-entry`, so the server shows Litty's icon and name.
    unsafe fn append_hints(&self, args: &mut DBusMessageIter) -> bool {
        let (mut hints, mut entry, mut value) = (DBusMessageIter::new(), DBusMessageIter::new(), DBusMessageIter::new());
        unsafe {
            (self.dbus_message_iter_open_container)(args, DBUS_TYPE_ARRAY, c"{sv}".as_ptr(), &mut hints) != 0
                && (self.dbus_message_iter_open_container)(&mut hints, DBUS_TYPE_DICT_ENTRY, std::ptr::null(), &mut entry) != 0
                && self.append_str(&mut entry, c"desktop-entry")
                && (self.dbus_message_iter_open_container)(&mut entry, DBUS_TYPE_VARIANT, c"s".as_ptr(), &mut value) != 0
                && self.append_str(&mut value, DESKTOP_ENTRY)
                && (self.dbus_message_iter_close_container)(&mut entry, &mut value) != 0
                && (self.dbus_message_iter_close_container)(&mut hints, &mut entry) != 0
                && (self.dbus_message_iter_close_container)(args, &mut hints) != 0
        }
    }

    /// A `Notify` call for `title` and `body`; null when libdbus runs out
    /// of memory (its only reason to refuse an append).
    unsafe fn notify_call(&self, title: &CStr, body: &CStr) -> *mut Msg {
        unsafe {
            let msg = (self.dbus_message_new_method_call)(
                NOTIFY_DEST.as_ptr(), NOTIFY_PATH.as_ptr(), NOTIFY_IFACE.as_ptr(), c"Notify".as_ptr(),
            );
            if msg.is_null() {
                return msg;
            }
            let mut args = DBusMessageIter::new();
            (self.dbus_message_iter_init_append)(msg, &mut args);
            let complete = self.append_str(&mut args, APP_NAME)
                && self.append_u32(&mut args, 0) // replaces_id: a new notification
                && self.append_str(&mut args, c"") // app_icon: the desktop entry's
                && self.append_str(&mut args, title)
                && self.append_str(&mut args, body)
                && self.append_actions(&mut args)
                && self.append_hints(&mut args)
                && self.append_i32(&mut args, -1); // expire_timeout: the server's default
            if !complete {
                (self.dbus_message_unref)(msg);
                return std::ptr::null_mut();
            }
            msg
        }
    }

    /// The `u32` at `iter`, if it holds one.
    ///
    /// # Safety
    /// `iter` must be a valid iterator into a live message.
    unsafe fn u32_at(&self, iter: &mut DBusMessageIter) -> Option<u32> {
        unsafe {
            if (self.dbus_message_iter_get_arg_type)(iter) != DBUS_TYPE_UINT32 {
                return None;
            }
            let mut value = 0u32;
            (self.dbus_message_iter_get_basic)(iter, (&mut value as *mut u32).cast());
            Some(value)
        }
    }

    /// What `msg` means for the notifications this connection sent.
    ///
    /// # Safety
    /// `msg` must be a live message.
    unsafe fn notification_incoming(&self, msg: *mut Msg) -> Incoming {
        unsafe {
            let kind = (self.dbus_message_get_type)(msg);
            if kind == DBUS_MESSAGE_TYPE_SIGNAL {
                return self.notification_signal(msg);
            }
            if !matches!(kind, DBUS_MESSAGE_TYPE_METHOD_RETURN | DBUS_MESSAGE_TYPE_ERROR) {
                return Incoming::Other;
            }
            let serial = (self.dbus_message_get_reply_serial)(msg);
            let mut iter = DBusMessageIter::new();
            if kind == DBUS_MESSAGE_TYPE_METHOD_RETURN
                && (self.dbus_message_iter_init)(msg, &mut iter) != 0
                && let Some(id) = self.u32_at(&mut iter)
            {
                return Incoming::Shown { serial, id };
            }
            Incoming::Failed { serial }
        }
    }

    /// `ActionInvoked(u, s)`, `ActivationToken(u, s)` or `NotificationClosed(u, u)`.
    ///
    /// # Safety
    /// `msg` must be a live signal.
    unsafe fn notification_signal(&self, msg: *mut Msg) -> Incoming {
        unsafe {
            let is = |member: &CStr| (self.dbus_message_is_signal)(msg, NOTIFY_IFACE.as_ptr(), member.as_ptr()) != 0;
            let mut iter = DBusMessageIter::new();
            if (self.dbus_message_iter_init)(msg, &mut iter) == 0 {
                return Incoming::Other;
            }
            let Some(id) = self.u32_at(&mut iter) else { return Incoming::Other };
            if is(c"NotificationClosed") {
                return Incoming::Closed { id };
            }
            if (self.dbus_message_iter_next)(&mut iter) == 0 {
                return Incoming::Other;
            }
            let Some(text) = self.string_at(&mut iter).and_then(|s| s.to_str().ok()) else {
                return Incoming::Other;
            };
            if is(c"ActionInvoked") {
                Incoming::Invoked { id, action: text.to_owned() }
            } else if is(c"ActivationToken") {
                Incoming::Token { id, token: text.to_owned() }
            } else {
                Incoming::Other
            }
        }
    }
}

struct Bus {
    lib: DBusLib,
    conn: *mut Conn,
    fd: RawFd,
    tracker: Tracker,
}

// The connection is only used under `BUS`'s lock.
unsafe impl Send for Bus {}

impl Bus {
    fn open() -> Option<Self> {
        let address = dbus::session_address()?;
        let lib = DBusLib::load()?;
        let (conn, fd) = dbus::open_session(&lib, &address)?;
        // No error out-parameter: the match is sent without waiting for the reply.
        unsafe { (lib.dbus_bus_add_match)(conn, SIGNAL_MATCH.as_ptr(), std::ptr::null_mut()) };
        Some(Self { lib, conn, fd, tracker: Tracker::default() })
    }

    /// Send `Notify` without waiting; the reply arrives through `dispatch`.
    fn send(&mut self, route: Route, title: &str, body: &str, c_title: &CStr, c_body: &CStr) -> io::Result<()> {
        let lib = &self.lib;
        let mut serial = 0u32;
        unsafe {
            let msg = lib.notify_call(c_title, c_body);
            if msg.is_null() {
                return Err(unavailable("notification message could not be built"));
            }
            let sent = (lib.dbus_connection_send)(self.conn, msg, &mut serial) != 0;
            (lib.dbus_message_unref)(msg);
            if !sent {
                return Err(unavailable("notification bus refused the message"));
            }
            // Flushing may also read incoming messages into libdbus's queue
            // without waking epoll; they are popped with the `Notify` reply,
            // which always follows on the socket.
            (lib.dbus_connection_flush)(self.conn);
        }
        self.tracker.sent(serial, route, title.to_owned(), body.to_owned());
        Ok(())
    }

    fn dispatch(&mut self, effects: &mut Vec<Effect>) -> Result<(), Disconnected> {
        unsafe { (self.lib.dbus_connection_read_write)(self.conn, 0) };
        loop {
            let msg = unsafe { (self.lib.dbus_connection_pop_message)(self.conn) };
            if msg.is_null() {
                break;
            }
            let incoming = unsafe { self.lib.notification_incoming(msg) };
            unsafe { (self.lib.dbus_message_unref)(msg) };
            effects.extend(self.tracker.on(incoming));
        }
        if unsafe { (self.lib.dbus_connection_get_is_connected)(self.conn) } == 0 {
            return Err(Disconnected);
        }
        Ok(())
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        unsafe {
            (self.lib.dbus_connection_close)(self.conn);
            (self.lib.dbus_connection_unref)(self.conn);
        }
    }
}

fn unavailable(why: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, why)
}

/// The notification bus connection closed.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Disconnected;

enum State {
    Unopened,
    Open(Box<Bus>),
    /// No bus, no watch, or the bus went away: `notify-send` from now on.
    Closed,
}

static BUS: Mutex<State> = Mutex::new(State::Unopened);

/// Show a notification whose click selects `pane` in the instance `origin`.
/// `Unsupported` when the session bus cannot carry it (the caller falls back
/// to `notify-send`); `InvalidInput` for text libdbus cannot take.
pub(in crate::platform::linux) fn show(origin: &str, pane: u64, title: &str, body: &str) -> io::Result<()> {
    let text = |s: &str| CString::new(s).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "notification contains NUL"));
    let (c_title, c_body) = (text(title)?, text(body)?);
    let mut state = BUS.lock();
    if matches!(*state, State::Unopened) {
        *state = match Bus::open() {
            Some(bus) if super::app::watch_notification_bus(bus.fd) => State::Open(Box::new(bus)),
            _ => State::Closed,
        };
    }
    let State::Open(bus) = &mut *state else {
        return Err(unavailable("no notification bus"));
    };
    bus.send(Route { origin: origin.to_owned(), pane }, title, body, &c_title, &c_body)
}

/// The bus socket the event loop watches, while the bus is open.
pub(super) fn watch_fd() -> Option<RawFd> {
    match &*BUS.lock() {
        State::Open(bus) => Some(bus.fd),
        _ => None,
    }
}

/// Read what the bus sent, adding what to do to `effects`. `Err` once the
/// bus is gone: the caller stops watching the socket, then calls `close`.
pub(super) fn dispatch(effects: &mut Vec<Effect>) -> Result<(), Disconnected> {
    match &mut *BUS.lock() {
        State::Open(bus) => bus.dispatch(effects),
        _ => Err(Disconnected),
    }
}

/// Drop the connection; later notifications use `notify-send`. Calls the
/// server never answered become `Fallback` effects, so they are still shown.
pub(super) fn close(effects: &mut Vec<Effect>) {
    let mut state = BUS.lock();
    if let State::Open(bus) = &mut *state {
        bus.tracker.abandon(effects);
    }
    *state = State::Closed;
}
```

  **`native/mod.rs`:** add `pub(super) mod notify_bus;` after `pub mod monitor;`.

  **`native/app.rs`.** After `register_key_repeat_fd` (line 62), add:

```rust
/// Watch the notification bus socket (event 7); `false` when it cannot be
/// watched, so the caller closes the bus rather than leave replies unread.
pub fn watch_notification_bus(fd: RawFd) -> bool {
    watch_bus_fd(*EPOLL_FD.lock(), fd)
}

fn watch_bus_fd(epoll: Option<RawFd>, fd: RawFd) -> bool {
    let Some(epoll) = epoll else { return false };
    let mut ev = libc::epoll_event { events: libc::EPOLLIN as u32, u64: 7 };
    if unsafe { libc::epoll_ctl(epoll, libc::EPOLL_CTL_ADD, fd, &mut ev) } == 0 {
        return true;
    }
    let err = std::io::Error::last_os_error();
    eprintln!("litty: cannot watch the notification bus ({err}); notifications use notify-send");
    false
}
```

  In `run_native`, after `let mut redraws: Vec<WindowId> = Vec::new();`
  (line 314), add:

```rust
        let mut notification_effects: Vec<super::notify_bus::Effect> = Vec::new();
```

  After the `tag == 6` branch (lines 400–402), add:

```rust
                    } else if tag == 7 {
                        notification_bus_readable(epoll_fd, &mut notification_effects);
```

  (it goes before the closing `}` of the `if`/`else if` chain). After
  `portal_readable`, add:

```rust
/// Read the notification bus: show refused notifications with the helper
/// and route clicks to their pane. Stop watching a bus that went away, or
/// `EPOLLHUP` would spin the loop.
fn notification_bus_readable(epoll_fd: RawFd, effects: &mut Vec<super::notify_bus::Effect>) {
    if super::notify_bus::dispatch(effects).is_err() {
        // Deregister before the socket closes, so a reused fd number is never removed.
        if let Some(fd) = super::notify_bus::watch_fd() {
            unsafe { libc::epoll_ctl(epoll_fd, libc::EPOLL_CTL_DEL, fd, std::ptr::null_mut()) };
        }
        super::notify_bus::close(effects);
    }
    for effect in effects.drain(..) {
        apply_notification_effect(effect);
    }
}

/// Show a refused notification with the helper, or hand a click (and the
/// token the server sent with it) on to the notification router.
fn apply_notification_effect(effect: super::notify_bus::Effect) {
    match effect {
        super::notify_bus::Effect::Fallback { title, body } => {
            crate::platform::process::show_notification(&title, &body);
        }
        super::notify_bus::Effect::Click { origin, pane, token } => {
            if let Some(token) = token {
                super::wayland_activation::offer_activation_token(&token);
            }
            crate::platform::linux::deliver_notification_click(origin, pane);
        }
    }
}
```

  Effects are handled after `dispatch` has released the bus lock.

  **`linux/notifications.rs`.** Replace everything above the new tests with:

```rust
//! Linux desktop notifications go over the session bus (`notify_bus`), so a
//! click reaches the notification router and selects its pane;
//! `notify-send` remains the fallback without a bus or notification server.
use std::{io, sync::Arc};

type Handler = Arc<dyn Fn(String, u64) + Send + Sync>;
static HANDLER: parking_lot::Mutex<Option<Handler>> = parking_lot::Mutex::new(None);

pub fn set_notification_handler(handler: Option<Handler>) { *HANDLER.lock() = handler; }
/// Hand a click to the notification router; dropped when no router runs.
pub(crate) fn deliver_notification_click(origin: String, pane: u64) {
    let handler = HANDLER.lock().clone();
    if let Some(handler) = handler { handler(origin, pane); }
}
#[allow(dead_code)] // Native launch initialization is macOS-only.
pub fn init_notifications() -> io::Result<()> { Ok(()) }
#[allow(dead_code)] // Native notification cleanup is macOS-only.
pub fn shutdown_notifications() {}
/// A click on a Linux notification never relaunches the app.
pub fn startup_is_notification() -> bool { false }
#[allow(dead_code)] // Linux has no AppKit reopen callback.
pub fn consume_notification_reopen() -> bool { false }
fn valid_origin(origin: &str) -> bool {
    let pieces: Vec<_> = origin.split('-').collect();
    pieces.len() == 3 && pieces[0].len() == 16 && !pieces[1].is_empty() && pieces[1].len() <= 8 && pieces[2].len() == 32
        && pieces.iter().all(|piece| piece.bytes().all(|b| b.is_ascii_hexdigit()))
}
/// `Unsupported` without a notification bus: the caller falls back to `notify-send`.
pub fn show_pane_notification(origin: &str, pane: u64, title: &str, body: &str) -> io::Result<()> {
    if !valid_origin(origin) || pane == 0 || title.len() > 4096 || body.len() > 16384
        || title.contains('\0') || body.contains('\0')
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid notification payload"));
    }
    super::native::notify_bus::show(origin, pane, title, body)
}
```

  **`linux/activation.rs`.** Replace the stub tail (`NotificationRouter`,
  `start_notification_router`, `notify_pane`) with macOS lines 627–695,
  making these edits:
  - The doc on `NotificationRouter` becomes
    `/// Bounded notification forwarding runs independently of the event loop and grid locks.`
  - In `start_notification_router`, the `send(...)` call becomes
    `send(&root(false)?, &Origin::parse(origin)?, ActivationAction::Pane(pane), None)`.
    The click's token was already offered in this process.
  - In `notify_pane`, the comment
    `// Standalone binaries lack an application notification identity.`
    becomes
    `// No activation server (benchmarks) or no notification bus: shown without click routing.`

  **`src/app/mod.rs:611`:** `cfg!(target_os = "macos") && !bench_ram_mode`
  becomes `cfg!(any(target_os = "macos", target_os = "linux")) && !bench_ram_mode`.

- [x] **Step 4: Run, expect PASS.**
  - `cargo test --bin litty platform::linux`: tracker, message, boundary, handler,
    watch and router tests pass. Message tests skip without libdbus.
  - `cargo test`: full suite.
  - `cargo clippy --all-targets -- -D warnings`: clean.
  - Then the full per-commit gate, quoting each result: `cargo check`,
    `cargo check --target x86_64-unknown-linux-gnu`, `cargo test`,
    `cargo clippy --all-targets -- -D warnings`, `git diff --check`.

- [x] **Step 5: Commit checkpoint.** Ready to commit:
  `src/platform/linux/native/dbus.rs src/platform/linux/native/notify_bus.rs src/platform/linux/native/mod.rs src/platform/linux/native/app.rs src/platform/linux/notifications.rs src/platform/linux/activation.rs src/app/mod.rs`
  — "feat(linux): route notification clicks to their pane over d-bus".

---

### Task 5: Verification, E2E and docs

**Files:**
- Modify: `README.md:23-24`
- Modify: `docs/guide/updates-and-cli.md:18-31`
- Modify: `docs/guide/notifications.md:50-57`
- Modify: `ROADMAP.md:121`
- Modify: `docs/plans/0.14.0.md:140`
- Modify: `docs/plans/wayland-backend.md:231`
- Modify: this plan (tick the steps)

- [x] **Step 1: Full verification.** Run each command and quote the result:
  - `cargo check`
  - `cargo check --target x86_64-unknown-linux-gnu`
  - `cargo test`
  - `cargo clippy --all-targets -- -D warnings`
  - `git diff --check`

  Then confirm that macOS is untouched:
  `git diff --stat main -- src/platform/macos src/platform/activation.rs`
  must print nothing.

- [x] **Step 2: E2E on GNOME Wayland, release build.** Run
  `cargo build --release` and set `L=$PWD/target/release/litty`. Repeat steps
  2a–2c for both backends (unset, then `LITTY_BACKEND=wayland`):

  a. **Reuse.** Start `$L` from the app launcher (or a terminal), then from
     another terminal run `cd /tmp && $L +reuse`. Expect:
     - the command exits 0;
     - `pgrep -c -x litty` still shows one process;
     - a new window opens in the first instance with its shell in `/tmp`.

     Record whether it was focused. On Wayland, also run
     `XDG_ACTIVATION_TOKEN=bogus $L +reuse`: it must still open the window.
     An invalid token at most stops the compositor from focusing.
  b. **Notification click.** In pane B of a split, run
     `sleep 3; printf '\e]9;hello from B\a'`, then focus another app.
     **The user clicks the notification.** Expect Litty's window to come
     back with pane B selected. Record whether the compositor raised it.
  c. **Dismiss.** Close a notification without clicking it. Expect nothing
     to happen in Litty and no error on stderr.
  d. **Fallback**, once:

     ```bash
     DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent strace -f -e trace=execve -o /tmp/litty-trace.txt "$L"
     ```

     Then `printf '\e]9;fallback\a'` in it. Expect an `execve` of
     `notify-send` in the trace. The popup itself cannot appear, because
     `notify-send` uses the same broken address. If `strace` is missing,
     report the gap instead.
  e. **Budgets.**
     - `$L +bench-ram`: RSS after cleanup and leak delta stay within the
       2026-11-16 numbers (`docs/plans/0.14.0.md:133-135`) plus noise.
     - An idle instance: `pidstat -p <pid> 5 3` (or `/proc/<pid>/stat`
       ticks) shows 0% CPU after one notification has been shown.

- [x] **Step 3: Docs.**
  - **`README.md:23`** becomes:

    "- **Opt-in process reuse on macOS and Linux**: `litty +reuse` (or
    `--reuse`) opens a new window in the newest eligible Litty process at the
    invoking directory. Ordinary launches remain independent. On Wayland the
    launcher's activation token is passed along so the compositor can focus
    the new window. See [CLI commands](docs/guide/updates-and-cli.md)."
  - **`README.md`:** add after line 24:

    "- **Linux notification clicks**: OSC 9 / OSC 777 notifications go to the
    desktop's notification server over D-Bus. Clicking one brings back its
    window and selects the pane that sent it; the compositor's focus policy
    decides whether the window is raised. Without a session bus or
    notification server, `notify-send` shows them without click routing. See
    [notifications](docs/guide/notifications.md)."
  - **`docs/guide/updates-and-cli.md`:**
    - Rename the heading to `## Reuse an existing process`.
    - Run `grep -rn "reuse-an-existing-macos-process" docs README.md` and
      update each link it finds to `#reuse-an-existing-process`.
    - Replace "Window activation follows macOS focus policy. Reuse is
      macOS-only; invoking it on Linux reports an unsupported operation." with:

      "Window activation follows the desktop's focus policy. On Linux the
      registry lives in `$XDG_RUNTIME_DIR/litty-a1` (or
      `/tmp/litty-a1-<uid>` without a runtime directory), and on Wayland the
      command passes its launcher's activation token to the running process
      so the compositor can focus the new window."
  - **`docs/guide/notifications.md`:**
    - Rename the heading at line 50 to `## Standalone macOS binaries`.
    - Replace lines 56–57 with a `## Linux` section stating:
      - notifications go to `org.freedesktop.Notifications` over D-Bus;
      - a click restores the window and selects the originating pane in
        the process that sent it;
      - on Wayland the server's activation token is used to focus, and the
        compositor may still refuse focus;
      - `notify-send` is used, without click routing, when no session bus or
        notification server is available or the server refuses the
        notification;
      - the E2E results recorded in Step 2.
  - **`ROADMAP.md:121`:** append "Notification click-through and
    single-instance token forwarding were completed later in
    [v0.14.0](./docs/plans/0.14.0.md)."
  - **`docs/plans/0.14.0.md:140`:** change `- [ ]` to `- [x]` and add an
    indented note with the date, the branch, the tests added, and the
    Step 2 outcomes per backend, including any focus the compositor refused.
  - **`docs/plans/wayland-backend.md:231`:** tick it and add a one-line
    pointer to the 0.14.0 note.

- [x] **Step 4: Final gate** (global CLAUDE.md): — review round 1 found the Task 3 wire tests missing from 1836ed5; restored verbatim.
  1. Read the full diff, untracked files included.
  2. Rerun Step 1.
  3. Self-review each item: requirements, concurrency (`BUS` → `EPOLL_FD`
     lock order only, never the reverse), error paths, hot path, security,
     naming and tests.
  4. Run `code-review-strict`, then fix or report its findings.
  5. Repeat if anything changed.

- [x] **Step 5: Commit checkpoint.** Ready to commit:
  `README.md docs/guide/updates-and-cli.md docs/guide/notifications.md ROADMAP.md docs/plans/0.14.0.md docs/plans/wayland-backend.md docs/plans/2026-11-16-linux-notification-activation.md`
  — "docs(linux): document reuse and notification clicks on linux".

**Gaps the executor must report:**
- macOS was not compiled locally (no Apple target on this host).
- The `SO_PEERCRED` mismatch path cannot be exercised without a second UID.
- A `getrandom` failure cannot be exercised.
- `startup_token_peek` is covered only by E2E step 2a.
- The `WaylandWindow::activate` external-token branch is covered only by
  E2E steps 2a/2b (it needs a live compositor).
- The live bus path (`Bus::open`, `send`, `dispatch`, epoll tag 7) is covered
  only by E2E steps 2b–2d; the unit tests cover the message building, the
  classification, the tracker and the effect handling.
- `the_session_bus_opens_with_a_socket` and
  `without_an_event_loop_notifications_are_unsupported` open the real session
  bus when one exists (deterministic either way); say which ran.
- Whether the compositor granted focus in each E2E case.
- sway and KDE were not tested.
