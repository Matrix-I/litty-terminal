//! System clipboard through `pbcopy` / `pbpaste` or `wl-copy` / `xclip` / `xsel`,
//! bounded in concurrency, time, and size.

use crate::platform;

/// A clipboard helper command. pbcopy / pbpaste convert text with the locale's
/// encoding, and an app started from Finder has no `LANG`: without a UTF-8
/// locale every non-ASCII character would turn into `?`.
fn clipboard_command(bin: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(bin);
    if cfg!(target_os = "macos") {
        cmd.env("LC_ALL", "en_US.UTF-8");
    }
    cmd
}

static IN_FLIGHT_COPIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
const MAX_CONCURRENT_COPIES: usize = 2;

type Helpers = &'static [(&'static str, &'static [&'static str])];

pub(crate) fn copy_to_clipboard(text: &str) {
    if platform::clipboard_write_string(text) {
        return;
    }
    #[cfg(target_os = "macos")]
    let cmds: Helpers = &[("/usr/bin/pbcopy", &[])];
    #[cfg(not(target_os = "macos"))]
    let cmds: Helpers = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["-ib"]),
    ];
    copy_with(text, cmds);
}

/// Set the X11 / Wayland PRIMARY selection (middle-click paste). No-op on macOS.
pub(crate) fn copy_to_primary(text: &str) {
    if platform::primary_write_string(text) {
        return;
    }
    #[cfg(not(target_os = "macos"))]
    copy_with(text, &[
        ("wl-copy", &["--primary"]),
        ("xclip", &["-selection", "primary"]),
        ("xsel", &["-ip"]),
    ]);
    #[cfg(target_os = "macos")]
    let _ = text;
}

/// Read the PRIMARY selection. `None` on macOS or when no helper succeeds.
pub(crate) fn paste_from_primary() -> Option<String> {
    if let Some(text) = platform::primary_string() {
        return Some(text);
    }
    #[cfg(not(target_os = "macos"))]
    return read_clipboard_with(&[
        ("wl-paste", &["--primary", "-n"]),
        ("xclip", &["-selection", "primary", "-o"]),
        ("xsel", &["-op"]),
    ]);
    #[cfg(target_os = "macos")]
    None
}

fn copy_with(text: &str, cmds: Helpers) {
    if IN_FLIGHT_COPIES.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= MAX_CONCURRENT_COPIES {
        IN_FLIGHT_COPIES.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    let text = text.to_string();
    if std::thread::Builder::new()
        .name("litty-clipboard-copy".into())
        .stack_size(128 * 1024)
        .spawn(move || {
            struct Guard;
            impl Drop for Guard {
                fn drop(&mut self) {
                    IN_FLIGHT_COPIES.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
            let _guard = Guard;
            use std::io::Write;
            use std::process::Stdio;

            for (bin, args) in cmds {
                if let Ok(mut child) = clipboard_command(bin)
                    .args(*args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(text.as_bytes());
                    }
                    let _ = child.wait();
                    return;
                }
            }
        })
        .is_err()
    {
        IN_FLIGHT_COPIES.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}
static IN_FLIGHT_PASTES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
const MAX_CONCURRENT_PASTES: usize = 2;

pub(crate) fn paste_from_clipboard() -> Option<String> {
    if let Some(text) = platform::clipboard_string() {
        return Some(text);
    }

    #[cfg(target_os = "macos")]
    let cmds: &'static [(&'static str, &'static [&'static str])] = &[("/usr/bin/pbpaste", &[])];
    #[cfg(not(target_os = "macos"))]
    let cmds: &'static [(&'static str, &'static [&'static str])] = &[
        ("wl-paste", &["-n"]),
        ("xclip", &["-selection", "clipboard", "-o"]),
        ("xsel", &["-ob"]),
    ];
    read_clipboard_with(cmds)
}

const MAX_CLIPBOARD_BYTES: usize = 16 << 20;

/// Drain a nonblocking pipe while checking the child and deadline. Waiting for
/// exit before reading deadlocks when the helper fills its stdout pipe.
fn read_clipboard_child(command: &mut std::process::Command, deadline: std::time::Instant, limit: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).process_group(0).spawn().ok()?;
    let output = (|| {
        let mut stdout = child.stdout.take()?;
        let fd = stdout.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 { return None; }
        let mut output = Vec::new();
        let mut buf = [0; 16 * 1024];
        let mut eof = false;
        let mut status = None;
        loop {
            if std::time::Instant::now() >= deadline { return None; }
            if !eof {
                match stdout.read(&mut buf) {
                    Ok(0) => eof = true,
                    Ok(n) => {
                        if n > limit.saturating_sub(output.len()) { return None; }
                        output.extend_from_slice(&buf[..n]);
                        continue;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {},
                    Err(_) => return None,
                }
            }
            if status.is_none() { status = child.try_wait().ok()?; }
            if status.is_some_and(|s| !s.success()) { return None; }
            if eof && status.is_some() { return Some(output); }
            // EOF can precede process exit; avoid polling a permanently hung-up fd.
            let mut pfd = libc::pollfd { fd: if eof { -1 } else { fd }, events: libc::POLLIN, revents: 0 };
            let wait = deadline.saturating_duration_since(std::time::Instant::now()).as_millis().min(5) as i32;
            unsafe { libc::poll(&mut pfd, 1, wait); }
        }
    })();
    if output.is_none() {
        // Also close descendants which inherited stdout. No detached reader
        // thread can remain blocked on a pipe after the deadline.
        unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL); }
        let _ = child.kill();
    }
    let _ = child.wait();
    output
}

/// Run the clipboard helpers in order on a worker thread and return the first
/// successful output, waiting at most 500ms and killing any hung child process.
pub(crate) fn read_clipboard_with(cmds: &'static [(&'static str, &'static [&'static str])]) -> Option<String> {
    if IN_FLIGHT_PASTES.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= MAX_CONCURRENT_PASTES {
        IN_FLIGHT_PASTES.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    if std::thread::Builder::new()
        .name("litty-clipboard-paste".into())
        .stack_size(128 * 1024)
        .spawn(move || {
            struct Guard;
            impl Drop for Guard {
                fn drop(&mut self) {
                    IN_FLIGHT_PASTES.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
            let _guard = Guard;
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(450);
            for (bin, args) in cmds {
                if std::time::Instant::now() >= deadline { break; }
                if let Some(bytes) = read_clipboard_child(clipboard_command(bin).args(*args), deadline, MAX_CLIPBOARD_BYTES)
                    && let Ok(text) = String::from_utf8(bytes) {
                        let _ = tx.send(Some(text));
                        return;
                    }
            }
            let _ = tx.send(None);
        })
        .is_err()
    {
        IN_FLIGHT_PASTES.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        return None;
    }

    rx.recv_timeout(std::time::Duration::from_millis(500)).ok().flatten()
}

#[cfg(test)]
mod tests {
    #[test]
    fn clipboard_helpers_drain_large_output_and_reject_over_limit_atomically() {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!("litty-clipboard-pipe-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let data = "Việt Nam 🦀\n".repeat(100_000);
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
        file.write_all(data.as_bytes()).unwrap();
        drop(file);
        let read = |limit| super::read_clipboard_child(std::process::Command::new("/bin/cat").arg(&path), std::time::Instant::now() + std::time::Duration::from_secs(3), limit);
        assert_eq!(read(data.len()).unwrap(), data.as_bytes(), "payload exceeds pipe capacity");
        let argument: &'static str = Box::leak(path.to_str().unwrap().to_owned().into_boxed_str());
        let args: &'static [&'static str] = Box::leak(vec![argument].into_boxed_slice());
        let helpers: &'static [(&'static str, &'static [&'static str])] = Box::leak(vec![("/bin/cat", args)].into_boxed_slice());
        assert_eq!(super::read_clipboard_with(helpers).unwrap(), data, "production worker drains large pipes too");
        assert!(read(data.len() - 1).is_none(), "never return a truncated success");
        std::fs::remove_file(&path).unwrap();
        assert!(super::read_clipboard_child(std::process::Command::new("/bin/sh").args(["-c", "printf partial; exit 1"]), std::time::Instant::now() + std::time::Duration::from_secs(1), 1024).is_none());
        assert!(super::read_clipboard_child(std::process::Command::new("/bin/sh").args(["-c", "printf partial; exec sleep 5"]), std::time::Instant::now() + std::time::Duration::from_millis(25), 1024).is_none());
        let start = std::time::Instant::now();
        assert!(super::read_clipboard_child(std::process::Command::new("/bin/sh").args(["-c", "sleep 5 & exit 0"]), start + std::time::Duration::from_millis(25), 1024).is_none());
        assert!(start.elapsed() < std::time::Duration::from_secs(1), "inherited stdout must not strand a worker");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn direct_macos_clipboard_read_write() {
        let msg = "litty_test_clipboard_12345";
        assert!(crate::platform::clipboard_write_string(msg));
        assert_eq!(crate::platform::clipboard_string().as_deref(), Some(msg));
    }
}
