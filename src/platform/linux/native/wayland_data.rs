//! Incoming file drops. Clipboard and primary selection stay on the separate
//! smithay-clipboard queue attached to the same process-lifetime connection.
//!
//! Receive only local file URIs and negotiate Copy (a terminal never moves
//! files). Pipe I/O runs on bounded workers, without a dispatch or grid lock.
use std::io::{self, Read};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::Weak;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use smithay_client_toolkit::reexports::client::{Connection, Proxy};
use smithay_client_toolkit::reexports::client::protocol::wl_data_offer::WlDataOffer;

use super::wayland::WindowShared;
use crate::platform::gui::WindowEvent;

const URI_LIST: &str = "text/uri-list";
const MAX_DROP_BYTES: usize = 1 << 20;
const MAX_DROP_FILES: usize = 4096;
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(2);
static TRANSFERS: AtomicUsize = AtomicUsize::new(0);

/// The clipboard hands ownership of a dropped offer to this guard. It is
/// destroyed on success, timeout, quota exhaustion and worker-spawn failure.
struct Offer(WlDataOffer);
impl Drop for Offer {
    fn drop(&mut self) { self.0.destroy(); }
}
struct Drag {
    offer: Offer,
    target: Weak<WindowShared>,
}

struct TransferSlot;
impl TransferSlot {
    fn acquire() -> Option<Self> {
        let mut n = TRANSFERS.load(Ordering::Relaxed);
        while n < 4 {
            match TRANSFERS.compare_exchange_weak(n, n + 1, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return Some(Self),
                Err(current) => n = current,
            }
        }
        None
    }
}
impl Drop for TransferSlot {
    fn drop(&mut self) { TRANSFERS.fetch_sub(1, Ordering::Relaxed); }
}

pub(super) fn receive_drop(conn: Connection, offer: smithay_clipboard::DndOffer, target: Weak<WindowShared>) {
    let drag = Drag { offer: Offer(offer.inner().clone()), target };
    let Some(slot) = TransferSlot::acquire() else { return; };
    let mut fds = [-1; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } != 0 { return; }
    let (reader, writer) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    // The receiver is nonblocking; sources expect a blocking write end.
    if unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_SETFL, 0) } == -1 { return; }
    drag.offer.0.receive(URI_LIST.into(), writer.as_fd());
    drop(writer);
    let _ = std::thread::Builder::new().name("litty-wl-drop".into()).stack_size(128 * 1024).spawn(move || {
        let _slot = slot;
        let paths = read_drop(reader, MAX_DROP_BYTES, TRANSFER_TIMEOUT).and_then(|bytes| parse_uri_list(&bytes));
        if let Ok(paths) = paths
            && !paths.is_empty()
            && let Some(win) = drag.target.upgrade()
        {
            win.state.lock().events.extend(paths.into_iter().map(WindowEvent::DroppedFile));
            if drag.offer.0.version() >= 3 { drag.offer.0.finish(); }
        }
        drop(drag); // finish (on success) precedes destroy, including on errors.
        // Requests are flushed by the event loop too if its socket is full.
        let _ = conn.flush();
        super::app::wakeup_main_loop();
    });
}

fn read_drop(fd: OwnedFd, limit: usize, timeout: Duration) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut file = std::fs::File::from(fd);
    let mut bytes = Vec::new();
    let mut buf = [0; 8192];
    loop {
        if Instant::now() >= deadline { return Err(io::ErrorKind::TimedOut.into()); }
        match file.read(&mut buf) {
            Ok(0) => return Ok(bytes),
            Ok(n) if n <= limit.saturating_sub(bytes.len()) => bytes.extend_from_slice(&buf[..n]),
            Ok(_) => return Err(io::ErrorKind::InvalidData.into()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                let mut pfd = libc::pollfd { fd: file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
                let ms = deadline.saturating_duration_since(Instant::now()).as_millis().clamp(1, i32::MAX as u128) as i32;
                if unsafe { libc::poll(&mut pfd, 1, ms) } < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() != io::ErrorKind::Interrupted { return Err(error); }
                }
            }
            Err(e) => return Err(e),
        }
    }
}

fn parse_uri_list(bytes: &[u8]) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for line in bytes.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() || line.starts_with(b"#") { continue; }
        if let Some(path) = local_file_uri(line) {
            if paths.len() >= MAX_DROP_FILES { return Err(io::ErrorKind::InvalidData.into()); }
            paths.push(path);
        }
    }
    Ok(paths)
}

fn local_file_uri(uri: &[u8]) -> Option<PathBuf> {
    if !uri.get(..5)?.eq_ignore_ascii_case(b"file:") { return None; }
    let mut path = &uri[5..];
    if let Some(authority) = path.strip_prefix(b"//") {
        let slash = authority.iter().position(|&b| b == b'/')?;
        let host = &authority[..slash];
        if !host.is_empty() && !host.eq_ignore_ascii_case(b"localhost") { return None; }
        path = &authority[slash..];
    }
    if !path.starts_with(b"/") || path.iter().any(|b| matches!(b, b'?' | b'#')) { return None; }
    let mut decoded = Vec::with_capacity(path.len());
    let mut index = 0;
    while index < path.len() {
        let byte = if path[index] == b'%' {
            let hi = hex(*path.get(index + 1)?)?;
            let lo = hex(*path.get(index + 2)?)?;
            index += 3;
            hi * 16 + lo
        } else {
            let b = path[index];
            index += 1;
            b
        };
        // Control bytes cannot be safely represented by the terminal paste
        // path (including bracketed-paste escape sequences).
        if byte < 0x20 || byte == 0x7f { return None; }
        decoded.push(byte);
    }
    if let Ok(text) = std::str::from_utf8(&decoded)
        && text.chars().any(char::is_control)
    { return None; }
    Some(PathBuf::from(std::ffi::OsString::from_vec(decoded)))
}
fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn uri_list_decodes_local_files_comments_and_crlf() {
        let paths = parse_uri_list(b"# File manager\r\nfile:///tmp/a%20b.txt\r\n\nfile://LOCALHOST/tmp/ti%E1%BA%BFng.txt\nfile:/tmp/it's+here\n").unwrap();
        assert_eq!(paths, [PathBuf::from("/tmp/a b.txt"), PathBuf::from("/tmp/tiếng.txt"), PathBuf::from("/tmp/it's+here")]);
    }

    #[test]
    fn only_local_absolute_file_uris_without_control_bytes_are_accepted() {
        for uri in [
            "https://example.com/file", "file://remote/tmp/a", "file://user@localhost/tmp/a",
            "file:relative", "file:///a%", "file:///a%GG", "file:///a%00", "file:///a%1b[201~",
            "file:///a%0aecho%20oops", "file:///a%C2%9B201~", "file:///a?query", "file:///a#fragment", "file://localhost",
        ] {
            assert!(local_file_uri(uri.as_bytes()).is_none(), "{uri}");
        }
        assert_eq!(local_file_uri(b"FILE:///tmp/a%23b%3Fc"), Some(PathBuf::from("/tmp/a#b?c")));
        assert_eq!(parse_uri_list(b"https://example.com/a\nfile:///tmp/ok\n").unwrap(), [PathBuf::from("/tmp/ok")]);
    }

    #[test]
    fn uri_decoding_preserves_unix_filename_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let path = local_file_uri(b"file:///tmp/%ff").unwrap();
        assert_eq!(path.as_os_str().as_bytes(), b"/tmp/\xff");
    }

    #[test]
    fn too_many_files_are_rejected_as_one_transfer() {
        let list = b"file:///tmp/a\n".repeat(MAX_DROP_FILES + 1);
        assert_eq!(parse_uri_list(&list).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    fn pipe() -> (OwnedFd, std::fs::File) {
        let mut fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) }, 0);
        unsafe { (OwnedFd::from_raw_fd(fds[0]), std::fs::File::from_raw_fd(fds[1])) }
    }

    #[test]
    fn transfer_waits_for_eof_and_rejects_oversized_or_stalled_sources() {
        let (reader, mut writer) = pipe();
        writer.write_all(b"file:///tmp/a\r\n").unwrap();
        drop(writer);
        assert_eq!(read_drop(reader, 64, Duration::from_secs(1)).unwrap(), b"file:///tmp/a\r\n");

        let (reader, mut writer) = pipe();
        writer.write_all(b"too much").unwrap();
        drop(writer);
        assert_eq!(read_drop(reader, 4, Duration::from_secs(1)).unwrap_err().kind(), io::ErrorKind::InvalidData);

        let (reader, mut writer) = pipe();
        writer.write_all(b"partial").unwrap();
        assert_eq!(read_drop(reader, 64, Duration::from_millis(10)).unwrap_err().kind(), io::ErrorKind::TimedOut);
        drop(writer);
    }

    #[test]
    fn concurrent_transfers_are_bounded_and_slots_are_released() {
        let slots: Vec<_> = (0..4).map(|_| TransferSlot::acquire().unwrap()).collect();
        assert!(TransferSlot::acquire().is_none());
        drop(slots);
        assert!(TransferSlot::acquire().is_some());
    }
}
