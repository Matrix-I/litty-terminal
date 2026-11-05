//! Control socket for `litty +toggle-quick-terminal` (Unix, both platforms).
//! The per-platform global hotkey lives in `platform::{macos,linux}::hotkey`.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use super::{EventSink, PlatformEvent, SinkSlot};

/// What `litty +toggle-quick-terminal` sends.
const TOGGLE: &[u8] = b"toggle-quick-terminal\n";

/// Where hotkey presses go, set by [`listen`].
static EVENTS: SinkSlot = SinkSlot::new();

pub(super) fn toggle() {
    EVENTS.emit(PlatformEvent::ToggleQuickTerminal);
}

/// File name of the control socket inside its directory.
const SOCKET_NAME: &str = "litty.sock";

/// Where the control socket lives, and whether Litty may change that directory.
#[derive(Debug, PartialEq, Eq)]
enum SocketDir {
    /// `$XDG_RUNTIME_DIR`: provided by the OS, already private; never modified.
    Runtime(PathBuf),
    /// A directory only Litty uses; created and restricted to `0o700` on listen.
    Own(PathBuf),
}

impl SocketDir {
    fn path(&self) -> &std::path::Path {
        match self {
            Self::Runtime(p) | Self::Own(p) => p,
        }
    }
}

fn current_uid() -> u32 {
    // SAFETY: `getuid` has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// Pure resolution of [`SocketDir`] so it can be tested without touching the
/// environment. Never returns the shared temp directory itself: it is nested
/// in a per-user directory so `listen` never has to chmod `/tmp` (CWE-732).
fn socket_dir_from(
    xdg_runtime: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    temp: PathBuf,
    uid: u32,
) -> SocketDir {
    if let Some(dir) = xdg_runtime.filter(|d| !d.is_empty()) {
        return SocketDir::Runtime(PathBuf::from(dir));
    }
    // Darwin has no `$HOME/.cache` convention; its temp dir is per-user already,
    // but `TMPDIR` can point anywhere, so it is nested like the Linux fallback.
    if !cfg!(target_os = "macos")
        && let Some(home) = home.filter(|h| !h.is_empty())
    {
        return SocketDir::Own(PathBuf::from(home).join(".cache").join("litty"));
    }
    SocketDir::Own(temp.join(format!("litty-{uid}")))
}

fn socket_dir() -> SocketDir {
    socket_dir_from(
        std::env::var_os("XDG_RUNTIME_DIR"),
        std::env::var_os("HOME"),
        std::env::temp_dir(),
        current_uid(),
    )
}

/// The control socket: in `$XDG_RUNTIME_DIR` (private to the user), else a
/// user-private directory under the cache or temp directory.
pub fn socket_path() -> PathBuf {
    socket_dir().path().join(SOCKET_NAME)
}

/// Creates `dir` (and missing parents) as `0o700` and makes sure it is a real
/// directory owned by the current user, so a symlink or a directory pre-created
/// by someone else in a shared location is refused instead of followed.
fn prepare_private_dir(dir: &std::path::Path, uid: u32) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() || meta.uid() != uid {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "control socket directory is not a directory owned by the current user",
        ));
    }
    if meta.mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// `litty +toggle-quick-terminal`: ask the running instance to toggle.
pub fn send_toggle() -> std::io::Result<()> {
    UnixStream::connect(socket_path())?.write_all(TOGGLE)
}

/// Binds the socket and restricts it to the owner. The parent directory is
/// already `0o700`, so the window before `chmod` is not reachable by others;
/// this avoids a process-wide `umask` change racing with other threads (CWE-362).
fn bind_private(path: &std::path::Path) -> std::io::Result<UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    let listener = UnixListener::bind(path)?;
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    Ok(listener)
}

/// Remember the event sink for hotkey presses and serve the control socket
/// for the rest of the process. Another running instance keeps the socket; a
/// stale file left by a crash is replaced.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn listen(sink: EventSink) {
    EVENTS.set(sink);
    let dir = socket_dir();
    if matches!(dir, SocketDir::Own(_)) && prepare_private_dir(dir.path(), current_uid()).is_err() {
        return;
    }
    let path = dir.path().join(SOCKET_NAME);
    if UnixStream::connect(&path).is_ok() {
        return;
    }
    let _ = std::fs::remove_file(&path);
    let Ok(listener) = bind_private(&path) else { return };
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn bound_socket_is_owner_only() {
        let dir = std::env::temp_dir().join(format!("litty-sock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.join("litty.sock");
        let listener = bind_private(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        drop(listener);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(mode, 0o600);
    }


    fn mode_of(p: &std::path::Path) -> u32 {
        std::fs::metadata(p).unwrap().permissions().mode() & 0o7777
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("litty-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fallback_never_resolves_to_the_shared_temp_dir() {
        let tmp = PathBuf::from("/tmp");
        let dir = socket_dir_from(None, None, tmp.clone(), 501);
        assert_eq!(dir, SocketDir::Own(tmp.join("litty-501")));
        let home = Some("/home/u".into());
        let SocketDir::Own(with_home) = socket_dir_from(None, home, tmp.clone(), 501) else {
            panic!("home fallback must be a directory Litty owns");
        };
        assert_ne!(with_home, tmp);
    }

    #[test]
    fn runtime_dir_is_used_as_is_and_marked_unmodifiable() {
        let dir = socket_dir_from(Some("/run/user/1000".into()), None, "/tmp".into(), 1000);
        assert_eq!(dir, SocketDir::Runtime("/run/user/1000".into()));
        assert!(matches!(socket_dir_from(Some("".into()), None, "/tmp".into(), 1), SocketDir::Own(_)));
    }

    #[test]
    fn preparing_a_nested_dir_leaves_the_shared_parent_untouched() {
        let shared = scratch("shared");
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1777)).unwrap();
        let own = shared.join("litty-501");
        prepare_private_dir(&own, current_uid()).unwrap();
        let (shared_mode, own_mode) = (mode_of(&shared), mode_of(&own));
        let _ = std::fs::remove_dir_all(&shared);
        assert_eq!(shared_mode, 0o1777);
        assert_eq!(own_mode, 0o700);
    }

    #[test]
    fn existing_loose_own_dir_is_tightened() {
        let dir = scratch("loose");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        prepare_private_dir(&dir, current_uid()).unwrap();
        let mode = mode_of(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn symlinked_dir_is_rejected_and_its_target_unchanged() {
        let base = scratch("link");
        let target = base.join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        let link = base.join("litty-501");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let result = prepare_private_dir(&link, current_uid());
        let target_mode = mode_of(&target);
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(target_mode, 0o755);
    }

    #[test]
    fn dir_owned_by_someone_else_is_rejected() {
        let dir = scratch("foreign");
        let other_uid = current_uid().wrapping_add(1);
        let result = prepare_private_dir(&dir, other_uid);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn toggle_request_reaches_the_event_sink() {
        let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let log = seen.clone();
        EVENTS.set(std::sync::Arc::new(move |e| log.lock().push(e)));
        toggle();
        assert_eq!(*seen.lock(), [PlatformEvent::ToggleQuickTerminal]);
    }

    #[test]
    fn listen_does_not_touch_process_umask() {
        // umask is process-wide: changing it races with concurrent file creation.
        let source = include_str!("control_socket.rs");
        let code = source.split("#[cfg(test)]").next().unwrap();
        assert!(!code.contains("libc::umask"));
    }
}
