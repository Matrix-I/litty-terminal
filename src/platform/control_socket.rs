//! Control socket for `litty +toggle-quick-terminal` (Unix, both platforms).
//! The per-platform global hotkey lives in `platform::{macos,linux}::hotkey`.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use winit::event_loop::EventLoopProxy;

use crate::keymap::Action;
use crate::AppEvent;

/// What `litty +toggle-quick-terminal` sends.
const TOGGLE: &[u8] = b"toggle-quick-terminal\n";

/// Where hotkey presses go, set by [`listen`].
static PROXY: parking_lot::Mutex<Option<EventLoopProxy<AppEvent>>> = parking_lot::const_mutex(None);

pub(super) fn toggle() {
    if let Some(proxy) = PROXY.lock().as_ref() {
        let _ = proxy.send_event(AppEvent::Menu(Action::ToggleQuickTerminal));
    }
}

/// The control socket: in `$XDG_RUNTIME_DIR` (private to the user), else user-private temp/cache directory.
pub fn socket_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join("litty.sock");
    }
    #[cfg(target_os = "macos")]
    {
        // Darwin temp_dir is user-private (/var/folders/.../T/)
        std::env::temp_dir().join("litty.sock")
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
            let dir = PathBuf::from(home).join(".cache").join("litty");
            let _ = std::fs::create_dir_all(&dir);
            dir.join("litty.sock")
        } else {
            std::env::temp_dir().join(format!("litty-{}.sock", unsafe { libc::getuid() }))
        }
    }
}

/// `litty +toggle-quick-terminal`: ask the running instance to toggle.
pub fn send_toggle() -> std::io::Result<()> {
    UnixStream::connect(socket_path())?.write_all(TOGGLE)
}

/// Remember the event proxy for hotkey presses and serve the control socket
/// for the rest of the process. Another running instance keeps the socket; a
/// stale file left by a crash is replaced.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn listen(proxy: EventLoopProxy<AppEvent>) {
    *PROXY.lock() = Some(proxy);
    let path = socket_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
    }
    if UnixStream::connect(&path).is_ok() {
        return;
    }
    let _ = std::fs::remove_file(&path);
    // Set restrictive umask 0o177 so socket is created with 0o600 atomically (zero race window)
    let old_umask = unsafe { libc::umask(0o177) };
    let bound = UnixListener::bind(&path);
    unsafe { libc::umask(old_umask) };
    let Ok(listener) = bound else { return };
    let spawned = std::thread::Builder::new().name("litty-control".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut buf = [0u8; 64];
            let mut stream = stream;
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
            let n = stream.read(&mut buf).unwrap_or(0);
            if &buf[..n] == TOGGLE {
                toggle();
            }
        }
    });
    if spawned.is_err() {
        let _ = std::fs::remove_file(&path);
    }
}
