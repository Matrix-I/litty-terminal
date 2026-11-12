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

const COPY_QUEUE_CAPACITY: usize = 16;
struct CopyRequest { text: String, helpers: Helpers }
struct CopyQueue { sender: std::sync::mpsc::SyncSender<CopyRequest> }
impl CopyQueue {
    fn start() -> std::io::Result<Self> { Self::start_with(run_copy) }

    fn start_with(run: impl Fn(CopyRequest) + Send + 'static) -> std::io::Result<Self> {
        Self::start_using(run, |work| std::thread::Builder::new().name("litty-clipboard-copy".into()).stack_size(128 * 1024).spawn(work).map(|_| ()))
    }

    fn start_using(run: impl Fn(CopyRequest) + Send + 'static, spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> std::io::Result<()>) -> std::io::Result<Self> {
        let (sender, receiver) = std::sync::mpsc::sync_channel(COPY_QUEUE_CAPACITY);
        spawn(Box::new(move || { while let Ok(request) = receiver.recv() { run(request); } }))?;
        Ok(Self { sender })
    }

    fn enqueue(&self, text: &str, helpers: Helpers) -> Result<(), &'static str> {
        if text.len() > MAX_CLIPBOARD_BYTES { return Err("The selection exceeds the clipboard size limit."); }
        self.sender.try_send(CopyRequest { text: text.to_owned(), helpers }).map_err(|error| match error {
            std::sync::mpsc::TrySendError::Full(_) => "Clipboard is busy. Try copying again after pending copies finish.",
            std::sync::mpsc::TrySendError::Disconnected(_) => "The clipboard worker is unavailable.",
        })
    }
}

fn copy_with(text: &str, helpers: Helpers) {
    static QUEUE: std::sync::OnceLock<std::io::Result<CopyQueue>> = std::sync::OnceLock::new();
    let result = match QUEUE.get_or_init(CopyQueue::start) {
        Ok(queue) => queue.enqueue(text, helpers),
        Err(_) => Err("Unable to start the clipboard worker."),
    };
    if let Err(error) = result { platform::alert("Copy failed", error); }
}

fn run_copy(request: CopyRequest) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(450);
    for (bin, args) in request.helpers {
        if write_clipboard_child(clipboard_command(bin).args(*args), request.text.as_bytes(), deadline) { return; }
        if std::time::Instant::now() >= deadline { break; }
    }
    // Worker diagnostics never include clipboard contents or command arguments.
    eprintln!("litty: clipboard helper failed or timed out");
}

/// Nonblocking writes bound both a helper that stops reading and one that never exits.
fn write_clipboard_child(command: &mut std::process::Command, text: &[u8], deadline: std::time::Instant) -> bool {
    write_clipboard_child_using(command, text, deadline, std::time::Instant::now, |command| command.spawn())
}

fn write_clipboard_child_using(
    command: &mut std::process::Command,
    text: &[u8],
    deadline: std::time::Instant,
    now: impl Fn() -> std::time::Instant,
    spawn: impl FnOnce(&mut std::process::Command) -> std::io::Result<std::process::Child>,
) -> bool {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    let Ok(mut child) = spawn(command.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0)) else { return false; };
    let success = (|| {
        let mut stdin = child.stdin.take()?;
        let fd = stdin.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 { return None; }
        let mut written = 0;
        while written < text.len() {
            if now() >= deadline { return None; }
            match stdin.write(&text[written..]) {
                Ok(0) => return None,
                Ok(n) => written += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    let mut pfd = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
                    let wait = deadline.saturating_duration_since(now()).as_millis().min(5) as i32;
                    unsafe { libc::poll(&mut pfd, 1, wait); }
                }
                Err(_) => return None,
            }
        }
        drop(stdin);
        loop {
            if let Some(status) = child.try_wait().ok()? { return status.success().then_some(()); }
            if now() >= deadline { return None; }
            let wait = deadline.saturating_duration_since(now()).as_millis().min(5) as i32;
            unsafe { libc::poll(std::ptr::null_mut(), 0, wait); }
        }
    })().is_some();
    if !success {
        unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL); }
        let _ = child.kill();
    }
    let _ = child.wait();
    success
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
    fn clipboard_accepts_a_copy_while_two_helpers_are_busy() {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let queue = super::CopyQueue::start_with(move |request| {
            entered_tx.send(request.text).unwrap();
            release_rx.recv().unwrap();
        }).unwrap();
        queue.enqueue("first", &[]).unwrap();
        assert_eq!(entered_rx.recv().unwrap(), "first");
        queue.enqueue("second", &[]).unwrap();
        queue.enqueue("queued copy", &[]).expect("a normal third copy must be queued rather than silently discarded");
        release_tx.send(()).unwrap();
        assert_eq!(entered_rx.recv().unwrap(), "second");
        release_tx.send(()).unwrap();
        assert_eq!(entered_rx.recv().unwrap(), "queued copy");
        release_tx.send(()).unwrap();
    }

    #[test]
    fn clipboard_worker_spawn_failure_is_reported() {
        let result = super::CopyQueue::start_using(|_| panic!("worker must not run"), |_| Err(std::io::Error::other("injected spawn failure")));
        assert!(matches!(result, Err(error) if error.to_string() == "injected spawn failure"));
    }

    #[test]
    fn clipboard_reports_full_disconnected_and_oversized_requests() {
        let (sender, receiver) = std::sync::mpsc::sync_channel(super::COPY_QUEUE_CAPACITY);
        let queue = super::CopyQueue { sender };
        for _ in 0..super::COPY_QUEUE_CAPACITY { queue.enqueue("queued", &[]).unwrap(); }
        assert!(queue.enqueue("rejected", &[]).unwrap_err().contains("busy"));
        drop(receiver);
        assert!(queue.enqueue("rejected", &[]).unwrap_err().contains("unavailable"));
        assert!(queue.enqueue(&"x".repeat(super::MAX_CLIPBOARD_BYTES + 1), &[]).unwrap_err().contains("size limit"));
    }

    #[test]
    fn clipboard_write_kills_and_reaps_helpers_blocked_during_or_after_input() {
        use std::cell::Cell;
        use std::time::{Duration, Instant};
        for (script, text, stop_before_write) in [
            ("kill -STOP $$", vec![b'x'; 1 << 20], true),
            ("cat >/dev/null; kill -STOP $$", b"complete input".to_vec(), false),
        ] {
            let pid = Cell::new(0);
            let calls = Cell::new(0);
            let base = Instant::now();
            let deadline = base + Duration::from_millis(20);
            let now = || {
                let call = calls.get();
                calls.set(call + 1);
                if !stop_before_write && call == 1 {
                    let mut status = 0;
                    assert_eq!(unsafe { libc::waitpid(pid.get(), &mut status, libc::WUNTRACED) }, pid.get());
                    assert!(libc::WIFSTOPPED(status));
                    return deadline;
                }
                base + Duration::from_millis(call * 5)
            };
            let spawn = |command: &mut std::process::Command| {
                let child = command.spawn()?;
                pid.set(child.id() as libc::pid_t);
                if stop_before_write {
                    let mut status = 0;
                    assert_eq!(unsafe { libc::waitpid(pid.get(), &mut status, libc::WUNTRACED) }, pid.get());
                    assert!(libc::WIFSTOPPED(status));
                }
                Ok(child)
            };
            assert!(!super::write_clipboard_child_using(std::process::Command::new("/bin/sh").args(["-c", script]), &text, deadline, now, spawn));
            let mut status = 0;
            assert_eq!(unsafe { libc::waitpid(pid.get(), &mut status, libc::WNOHANG) }, -1);
            assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ECHILD), "helper must already be reaped");
        }
    }

    #[test]
    fn clipboard_write_handles_large_input_exit_failure_and_expired_deadline() {
        let path = std::env::temp_dir().join(format!("litty-copy-write-{}", std::process::id()));
        let text = "Việt Nam 🦀\n".repeat(100_000);
        assert!(super::write_clipboard_child(std::process::Command::new("/usr/bin/tee").arg(&path), text.as_bytes(), std::time::Instant::now() + std::time::Duration::from_secs(3)));
        assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
        std::fs::remove_file(path).unwrap();
        assert!(!super::write_clipboard_child(&mut std::process::Command::new("/usr/bin/false"), b"text", std::time::Instant::now() + std::time::Duration::from_secs(3)));
        assert!(!super::write_clipboard_child(&mut std::process::Command::new("/usr/bin/true"), b"text", std::time::Instant::now()));
        assert!(!super::write_clipboard_child(&mut std::process::Command::new("/litty-nonexistent-clipboard-helper"), b"text", std::time::Instant::now()));
    }

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
