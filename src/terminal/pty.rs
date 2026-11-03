//! POSIX pseudo-terminal: spawns the user's shell on the slave side and runs
//! the reader thread that parses output straight into the shared grid.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::mpsc;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::terminal::grid::Grid;

#[derive(Debug)]
pub(crate) enum WritePayload {
    Inline(u8, [u8; 15]),
    Heap(Vec<u8>),
}

impl WritePayload {
    #[inline]
    pub fn from_slice(data: &[u8]) -> Self {
        if data.len() <= 15 {
            let mut buf = [0u8; 15];
            buf[..data.len()].copy_from_slice(data);
            WritePayload::Inline(data.len() as u8, buf)
        } else {
            WritePayload::Heap(data.to_vec())
        }
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        match self {
            WritePayload::Inline(len, buf) => &buf[..*len as usize],
            WritePayload::Heap(v) => v.as_slice(),
        }
    }
}

/// Atomic nonblocking enqueue: the caller can report/retry the entire rejected
/// payload rather than silently losing a key or a tail of pasted input.
fn try_write(tx: &mpsc::SyncSender<WritePayload>, data: &[u8]) -> io::Result<()> {
    tx.try_send(WritePayload::from_slice(data)).map_err(|e| match e {
        mpsc::TrySendError::Full(_) => io::Error::new(io::ErrorKind::WouldBlock, "PTY input queue is full; input was not sent"),
        mpsc::TrySendError::Disconnected(_) => io::Error::new(io::ErrorKind::BrokenPipe, "PTY writer has stopped"),
    })
}

/// Reader-only backpressure. Keep the reply owned until it can enter the same
/// FIFO as input, polling the shutdown pipe so closing a stalled pane exits.
/// This never runs on the UI thread or while holding a grid guard.
fn queue_reply(tx: &mpsc::SyncSender<WritePayload>, mut data: WritePayload, shutdown: RawFd) -> io::Result<()> {
    loop {
        match tx.try_send(data) {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Disconnected(_)) => return Err(io::Error::new(io::ErrorKind::BrokenPipe, "PTY writer has stopped")),
            Err(mpsc::TrySendError::Full(pending)) => data = pending,
        }
        let mut fd = libc::pollfd { fd: shutdown, events: libc::POLLIN, revents: 0 };
        let ready = unsafe { libc::poll(&mut fd, 1, 2) };
        if ready > 0 { return Err(io::Error::new(io::ErrorKind::BrokenPipe, "PTY closed during reply backpressure")); }
        if ready < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted { return Err(io::Error::last_os_error()); }
    }
}

/// Dropping a `Pty` closes the master side and wakes both I/O threads through the
/// shutdown pipe; once their duplicated descriptors are closed the kernel hangs up
/// the terminal (SIGHUP to the shell and its foreground job) and the reader thread
/// reaps the child, so no zombie is left behind.
pub struct Pty {
    master: OwnedFd,
    pub pid: libc::pid_t,
    /// Bytes for the shell, written by the writer thread so the UI never blocks.
    tx: mpsc::SyncSender<WritePayload>,
    /// Read end of the shutdown pipe, polled by the I/O threads.
    shutdown_rx: Arc<OwnedFd>,
    /// Write end of the shutdown pipe: closing it (on drop) signals shutdown.
    _shutdown_tx: OwnedFd,
    /// Must stay the last field: fields drop in declaration order, so the master,
    /// the writer's channel and the shutdown pipe are released before it waits.
    reaper: ReapOnDrop,
}

/// Reaps the shell when the owning `Pty` is dropped, unless the reader thread was
/// spawned (which takes over reaping the child). Waiting from `Pty::drop` itself would
/// block while the master is still open: macOS then keeps the exiting shell stuck
/// draining unread tty output, so the wait never returns.
struct ReapOnDrop {
    pid: libc::pid_t,
    reader_spawned: std::sync::atomic::AtomicBool,
}

impl Drop for ReapOnDrop {
    fn drop(&mut self) {
        if !self.reader_spawned.load(std::sync::atomic::Ordering::SeqCst) {
            reap_child(self.pid);
        }
    }
}

/// Stop at every image boundary. Decode/file I/O runs on this reader thread
/// with no grid guard; following bytes cannot overtake the result. Keeping the
/// job local also bounds outstanding work to one operation per pane.
fn feed_output(
    grid: &Mutex<Grid>, parser: &mut vte::Parser, bytes: &[u8], reply: &mut Vec<u8>,
    mut decode: impl FnMut(crate::terminal::grid::ImageJob) -> crate::terminal::grid::ImageCompletion,
) -> bool {
    let mut offset = 0;
    loop {
        let mut g = grid.lock();
        g.defer_images = true;
        while offset < bytes.len() && g.image_job.is_none() {
            offset += g.feed_bytes(parser, &bytes[offset..]);
        }
        g.defer_images = false;
        if let Some(job) = g.image_job.take() {
            let epoch = g.image_epoch;
            drop(g);
            let result = decode(job);
            let mut g = grid.lock();
            // A UI clear/reset during decoding cancels the old image.
            if g.image_epoch == epoch { g.complete_image(result); }
            drop(g);
        } else {
            reply.append(&mut g.response);
            return g.modes.sync_output;
        }
    }
}

fn winsize(cols: u16, rows: u16, cell_w: u32, cell_h: u32) -> libc::winsize {
    libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: (cols as u32 * cell_w).min(u16::MAX as u32) as u16,
        ws_ypixel: (rows as u32 * cell_h).min(u16::MAX as u32) as u16,
    }
}

fn cstr(s: impl Into<Vec<u8>>) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new("?").unwrap())
}

/// Shell integration for `shell_name`, unless disabled (`None`).
fn shell_integration_for(
    shell_name: &str,
    kind: Option<crate::terminal::shell_integration::ShellIntegration>,
) -> Option<crate::terminal::shell_integration::Injection> {
    crate::terminal::shell_integration::prepare(shell_name, kind?, |k| std::env::var(k).ok())
}

impl Pty {
    pub fn spawn_in(
        cols: u16,
        rows: u16,
        cell_w: u32,
        cell_h: u32,
        cwd: Option<&str>,
        integration: Option<crate::terminal::shell_integration::ShellSetup>,
    ) -> io::Result<Pty> {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| if cfg!(target_os = "macos") { "/bin/zsh".into() } else { "/bin/bash".into() });
        Self::spawn_command(&shell, None, &[], cols, rows, cell_w, cell_h, cwd, integration)
    }

    /// Run `program` with `args` (argv[0] included) on a fresh PTY through the same
    /// fork path as a shell, without shell integration. Used by the PTY benchmark.
    pub(crate) fn spawn_program(
        program: &str,
        args: &[&str],
        env: &[(&str, &str)],
        cols: u16,
        rows: u16,
    ) -> io::Result<Pty> {
        Self::spawn_command(program, Some(args), env, cols, rows, 8, 16, None, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_command(
        shell: &str,
        argv_override: Option<&[&str]>,
        extra_env: &[(&str, &str)],
        cols: u16,
        rows: u16,
        cell_w: u32,
        cell_h: u32,
        cwd: Option<&str>,
        integration: Option<crate::terminal::shell_integration::ShellSetup>,
    ) -> io::Result<Pty> {
        let shell_name = shell.rsplit('/').next().unwrap_or("sh").to_string();

        // Everything the child needs is prepared before fork(): after fork only
        // async-signal-safe calls are allowed.
        let path = cstr(shell);
        let injection = shell_integration_for(&shell_name, integration.as_ref().map(|(kind, _, _)| *kind));
        // A leading '-' in argv[0] makes a login shell; injected bash emulates one.
        let args: Vec<CString> = match (argv_override, injection.as_ref().and_then(|i| i.argv.as_ref())) {
            (Some(a), _) => a.iter().map(|s| cstr(*s)).collect(),
            (None, Some(a)) => a.iter().map(|s| cstr(s.as_str())).collect(),
            (None, None) => vec![cstr(format!("-{shell_name}"))],
        };
        let mut argv: Vec<*const libc::c_char> = args.iter().map(|a| a.as_ptr()).collect();
        argv.push(std::ptr::null());
        let injected_env = injection.map(|i| i.env).unwrap_or_default();

        let target_dir = cwd
            .filter(|d| !d.is_empty() && std::path::Path::new(d).is_dir())
            .map( String::from )
            .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".into()));

        let mut env: Vec<CString> = std::env::vars()
            .filter(|(k, _)| {
                !matches!(
                    k.as_str(),
                    "TERM" | "COLORTERM" | "TERM_PROGRAM" | "TERM_PROGRAM_VERSION" | "PWD" | "OLDPWD"
                ) && !injected_env.iter().any(|(ik, _)| ik == k)
                    && !k.starts_with("GHOSTTY_")
                    && !k.starts_with("KITTY_")
                    && !k.starts_with("ITERM_")
                    && !k.starts_with("WEZTERM_")
            })
            .map(|(k, v)| cstr(format!("{k}={v}")))
            .collect();
        env.push(cstr("TERM=xterm-256color"));
        env.push(cstr("COLORTERM=truecolor"));
        env.push(cstr("TERM_PROGRAM=litty"));
        env.push(cstr(concat!("TERM_PROGRAM_VERSION=", env!("CARGO_PKG_VERSION"))));
        for (k, v) in &injected_env {
            env.push(cstr(format!("{k}={v}")));
        }
        if let Some((_, feats, ai_trigger)) = &integration {
            env.push(cstr("LITTY_SHELL_INTEGRATION=1"));
            env.push(cstr(format!("LITTY_AI_TRIGGER={ai_trigger}")));
            let mut list = Vec::new();
            if feats.cursor { list.push("cursor"); }
            if feats.sudo { list.push("sudo"); }
            if feats.title { list.push("title"); }
            if !list.is_empty() {
                let joined = list.join(",");
                env.push(cstr(format!("LITTY_SHELL_INTEGRATION_FEATURES={joined}")));
            }
        }
        for (k, v) in extra_env {
            env.push(cstr(format!("{k}={v}")));
        }
        env.push(cstr(format!("PWD={target_dir}")));
        if std::env::var_os("LANG").is_none() && std::env::var_os("LC_ALL").is_none() {
            env.push(cstr("LANG=en_US.UTF-8"));
        }
        let mut envp: Vec<*const libc::c_char> = env.iter().map(|s| s.as_ptr()).collect();
        envp.push(std::ptr::null());
        let start_dir = cstr(target_dir);
        let mut rlim: libc::rlimit = unsafe { std::mem::zeroed() };
        let rlim_max = if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) } == 0 && rlim.rlim_cur > 0 {
            rlim.rlim_cur as libc::c_long
        } else {
            4096
        };
        let sys_max = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
        let _max_fd = sys_max.max(rlim_max).clamp(256, 65536) as libc::c_int;

        let mut ws = winsize(cols, rows, cell_w, cell_h);
        let (mut master, mut slave): (libc::c_int, libc::c_int) = (-1, -1);
        // SAFETY: plain libc FFI with valid pointers.
        unsafe {
            if libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut::<libc::termios>(),
                // `*mut winsize` on macOS, `*const winsize` on Linux.
                std::ptr::addr_of_mut!(ws) as _,
            ) != 0
            {
                return Err(io::Error::last_os_error());
            }

            // UTF-8 aware line editing (correct backspace over multibyte chars).
            let mut tio: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(slave, &mut tio) == 0 {
                tio.c_iflag |= libc::IUTF8;
                libc::tcsetattr(slave, libc::TCSANOW, &tio);
            }

            #[cfg(target_os = "macos")]
            let open_fds: Vec<libc::c_int> = std::fs::read_dir("/dev/fd")
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|e| e.file_name().to_string_lossy().parse::<libc::c_int>().ok())
                        .filter(|&fd| fd >= 3)
                        .collect()
                })
                .unwrap_or_default();

            let pid = libc::fork();
            if pid < 0 {
                let e = io::Error::last_os_error();
                libc::close(master);
                libc::close(slave);
                return Err(e);
            }
            if pid == 0 {
                // ---- child ----
                libc::setsid();
                libc::ioctl(slave, libc::TIOCSCTTY as _, 0);
                libc::dup2(slave, 0);
                libc::dup2(slave, 1);
                libc::dup2(slave, 2);
                if slave > 2 {
                    libc::close(slave);
                }
                libc::close(master);
                // Close all inherited file descriptors >= 3 so no PTY masters, sockets,
                // or driver handles leak into the shell.
                #[cfg(all(target_os = "linux", target_env = "gnu"))]
                if libc::close_range(3, !0, 0) != 0 {
                    for fd in 3.._max_fd {
                        libc::close(fd);
                    }
                }
                #[cfg(any(
                    target_os = "freebsd",
                    target_os = "openbsd",
                    target_os = "netbsd",
                    target_os = "dragonfly"
                ))]
                libc::closefrom(3);
                #[cfg(target_os = "macos")]
                {
                    if !open_fds.is_empty() {
                        for &fd in &open_fds {
                            libc::close(fd);
                        }
                    } else {
                        for fd in 3.._max_fd {
                            libc::close(fd);
                        }
                    }
                }
                #[cfg(not(any(
                    all(target_os = "linux", target_env = "gnu"),
                    target_os = "macos",
                    target_os = "freebsd",
                    target_os = "openbsd",
                    target_os = "netbsd",
                    target_os = "dragonfly"
                )))]
                for fd in 3.._max_fd {
                    libc::close(fd);
                }
                // Rust ignores SIGPIPE; ignored dispositions survive exec, so reset them.
                for sig in [libc::SIGPIPE, libc::SIGINT, libc::SIGQUIT, libc::SIGTERM, libc::SIGCHLD, libc::SIGHUP] {
                    libc::signal(sig, libc::SIG_DFL);
                }
                let mut set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::sigprocmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
                libc::chdir(start_dir.as_ptr());
                libc::execve(path.as_ptr(), argv.as_ptr(), envp.as_ptr());
                libc::_exit(127);
            }

            // ---- parent ----
            libc::close(slave);
            let master = OwnedFd::from_raw_fd(master);
            set_cloexec(master.as_raw_fd());
            // Non-blocking master: the I/O threads poll it together with the shutdown pipe.
            let flags = libc::fcntl(master.as_raw_fd(), libc::F_GETFL);
            libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);

            let mut pipe_fds = [-1; 2];
            if libc::pipe(pipe_fds.as_mut_ptr()) != 0 {
                let e = io::Error::last_os_error();
                drop(master);
                reap_child(pid);
                return Err(e);
            }
            let shutdown_rx = Arc::new(OwnedFd::from_raw_fd(pipe_fds[0]));
            let shutdown_tx = OwnedFd::from_raw_fd(pipe_fds[1]);
            set_cloexec(shutdown_rx.as_raw_fd());
            set_cloexec(shutdown_tx.as_raw_fd());

            let (tx, rx) = mpsc::sync_channel::<WritePayload>(2048);
            let writer_fd = match dup_cloexec(master.as_raw_fd()) {
                Ok(fd) => fd,
                Err(e) => {
                    drop(master);
                    reap_child(pid);
                    return Err(e);
                }
            };
            let writer_shutdown = shutdown_rx.clone();
            if let Err(e) = std::thread::Builder::new()
                .name("pty-writer".into())
                .stack_size(64 * 1024)
                .spawn(move || {
                    while let Ok(data) = rx.recv() {
                        if !write_all(writer_fd.as_raw_fd(), data.as_slice(), writer_shutdown.as_raw_fd()) {
                            return;
                        }
                    }
                })
            {
                drop(master);
                reap_child(pid);
                return Err(e);
            }

            Ok(Pty {
                master,
                pid,
                tx,
                shutdown_rx,
                _shutdown_tx: shutdown_tx,
                reaper: ReapOnDrop { pid, reader_spawned: std::sync::atomic::AtomicBool::new(false) },
            })
        }
    }

    /// Query the current working directory of the foreground process group (or
    /// the login shell itself) directly from the OS kernel.
    /// A program other than the shell owns the terminal (the foreground
    /// process group is not the shell's).
    pub fn has_running_process(&self) -> bool {
        // SAFETY: `tcgetpgrp` only reads the terminal's foreground group.
        let pgrp = unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) };
        pgrp > 0 && pgrp != self.pid
    }

    pub fn cwd(&self) -> Option<String> {
        let pgrp = unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) };
        if pgrp > 0 && pgrp != self.pid
            && let Some(dir) = pid_cwd(pgrp) {
                return Some(dir);
            }
        pid_cwd(self.pid)
    }

    pub fn resize(&self, cols: u16, rows: u16, cell_w: u32, cell_h: u32) {
        let ws = winsize(cols, rows, cell_w, cell_h);
        // SAFETY: valid fd and pointer; the kernel sends SIGWINCH to the shell.
        unsafe {
            libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &ws);
        }
    }

    /// Queue bytes for the shell. Uses WritePayload to avoid heap allocation on keystrokes.
    pub fn write(&self, data: &[u8]) -> io::Result<()> {
        try_write(&self.tx, data)
    }

    /// Spawn Thread 1: reads into a fixed 64 KB stack buffer, parsed by `vte`
    /// directly into the locked grid. Coalesces back-to-back shell writes
    /// (e.g. `\r\n` + `PROMPT_SP` + multi-line Powerline prompt) so intermediate
    /// incomplete states don't trigger redundant GPU frames. When the shell exits
    /// or the `Pty` is dropped, the thread closes its descriptor and reaps the child.
    pub fn spawn_reader(
        &self,
        grid: Arc<Mutex<Grid>>,
        notify: impl Fn() + Send + 'static,
        on_exit: impl FnOnce() + Send + 'static,
    ) -> io::Result<()> {
        let fd = match dup_cloexec(self.master.as_raw_fd()) {
            Ok(fd) => fd,
            Err(e) => {
                reap_child(self.pid);
                return Err(e);
            }
        };
        let shutdown = self.shutdown_rx.clone();
        let tx = self.tx.clone();
        let pid = self.pid;
        struct ChildReaper(libc::pid_t);
        impl Drop for ChildReaper {
            fn drop(&mut self) {
                reap_child(self.0);
            }
        }
        if let Err(e) = std::thread::Builder::new()
            .name("pty-reader".into())
            .stack_size(256 * 1024)
            .spawn(move || {
                let _reaper = ChildReaper(pid);
                let raw = fd.as_raw_fd();
                let mut buf = [0u8; 65536];
                let mut parser = vte::Parser::new();
                let mut reply: Vec<u8> = Vec::with_capacity(256);
                'read: loop {
                    if !wait_ready(raw, libc::POLLIN, shutdown.as_raw_fd()) {
                        break; // Pty dropped
                    }
                    // SAFETY: buf is a valid writable buffer of buf.len() bytes.
                    let n = unsafe { libc::read(raw, buf.as_mut_ptr().cast(), buf.len()) };
                    if n < 0 {
                        let kind = io::Error::last_os_error().kind();
                        if matches!(kind, io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock) {
                            continue;
                        }
                        break; // EIO: slave closed (shell exited)
                    }
                    if n == 0 {
                        break;
                    }

                    let mut chunk_len = n as usize;
                    let batch_start = std::time::Instant::now();
                    const FEED_CHUNK_SIZE: usize = 8192;
                    loop {
                        let mut sync_active = false;
                        for chunk in buf[..chunk_len].chunks(FEED_CHUNK_SIZE) {
                            sync_active = feed_output(&grid, &mut parser, chunk, &mut reply, |job| job.decode());
                        }
                        if !reply.is_empty() {
                            // Same queue as keyboard input, so replies never interleave with a paste.
                            notify();
                            if let Err(e) = queue_reply(&tx, WritePayload::Heap(std::mem::take(&mut reply)), shutdown.as_raw_fd()) {
                                eprintln!("litty: terminal reply failed: {e}");
                                break 'read;
                            }
                        }

                        // Never hold output back waiting for the next prompt: prompt themes take
                        // anywhere from a few ms to hundreds (e.g. git status), and holding the
                        // newline after Enter until then makes typing feel uneven. Only an
                        // application's synchronized-output frame (DECSET 2026) is waited for.
                        let max_batch_ms = if sync_active { 45 } else { 3 };
                        if batch_start.elapsed().as_millis() >= max_batch_ms {
                            break;
                        }
                        let wait_ms = if sync_active { 6 } else { 0 };
                        let mut pfd = libc::pollfd { fd: raw, events: libc::POLLIN, revents: 0 };
                        let pr = unsafe { libc::poll(&mut pfd, 1, wait_ms) };
                        if pr <= 0 || (pfd.revents & libc::POLLIN) == 0 {
                            break;
                        }
                        let n2 = unsafe { libc::read(raw, buf.as_mut_ptr().cast(), buf.len()) };
                        if n2 <= 0 {
                            break;
                        }
                        chunk_len = n2 as usize;
                    }
                    notify();
                }
                // Release our handles on the master first so the terminal can hang up
                // (the writer thread exits once every sender is gone).
                drop(tx);
                drop(fd);
                drop(_reaper);
                on_exit();
            })
        {
            reap_child(self.pid);
            return Err(e);
        }
        self.reaper.reader_spawned.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

fn set_cloexec(fd: RawFd) {
    // SAFETY: fcntl on a descriptor we own.
    unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
}

fn dup_cloexec(fd: RawFd) -> io::Result<OwnedFd> {
    // SAFETY: dup of a valid fd; the returned descriptor is owned by the caller.
    let new_fd = unsafe { libc::dup(fd) };
    if new_fd < 0 {
        return Err(io::Error::last_os_error());
    }
    set_cloexec(new_fd);
    Ok(unsafe { OwnedFd::from_raw_fd(new_fd) })
}

/// Block until `fd` reports `events` (or hangs up / errors). Returns `false` once the
/// shutdown pipe becomes readable or is closed.
fn wait_ready(fd: RawFd, events: libc::c_short, shutdown: RawFd) -> bool {
    loop {
        let mut fds = [
            libc::pollfd { fd, events, revents: 0 },
            libc::pollfd { fd: shutdown, events: libc::POLLIN, revents: 0 },
        ];
        // SAFETY: fds is a valid array of two pollfd structs.
        let r = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if r < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return false;
        }
        if fds[1].revents != 0 {
            return false;
        }
        if fds[0].revents != 0 {
            return true;
        }
    }
}

/// Write all of `data` to the non-blocking master, waiting for buffer space as needed.
/// Returns `false` if the terminal is gone or shutdown was requested.
fn write_all(fd: RawFd, mut data: &[u8], shutdown: RawFd) -> bool {
    while !data.is_empty() {
        // SAFETY: data is a valid readable slice.
        let n = unsafe { libc::write(fd, data.as_ptr().cast(), data.len()) };
        if n < 0 {
            match io::Error::last_os_error().kind() {
                io::ErrorKind::Interrupted => continue,
                io::ErrorKind::WouldBlock => {
                    if !wait_ready(fd, libc::POLLOUT, shutdown) {
                        return false;
                    }
                    continue;
                }
                _ => return false,
            }
        }
        data = &data[n as usize..];
    }
    true
}

/// Reap the shell. Give it a moment to exit on hangup, then SIGKILL so a shell that
/// ignores SIGHUP can never linger as an orphan. Safe to signal `pid` here: it cannot
/// be reused until it has been waited for, and this is the only place that waits.
fn reap_child(pid: libc::pid_t) {
    let mut status = 0;
    for _ in 0..40 {
        // SAFETY: plain libc FFI with a valid pointer.
        let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if r != 0 {
            return; // reaped, or not our child (ECHILD)
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    unsafe {
        libc::kill(pid, libc::SIGKILL);
        libc::waitpid(pid, &mut status, 0);
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn pid_cwd(pid: libc::pid_t) -> Option<String> {
    if pid <= 0 {
        return None;
    }
    unsafe {
        let mut info: libc::proc_vnodepathinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
        let ret = libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&mut info as *mut libc::proc_vnodepathinfo).cast::<libc::c_void>(),
            size,
        );
        if ret != size {
            return None;
        }
        if let Some(chunk) = info.pvi_cdir.vip_path.last_mut()
            && let Some(last) = chunk.last_mut() {
                *last = 0;
            }
        let cstr = std::ffi::CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr().cast::<libc::c_char>());
        let s = cstr.to_str().ok()?.to_string();
        (!s.is_empty()).then_some(s)
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn pid_cwd(pid: libc::pid_t) -> Option<String> {
    if pid <= 0 {
        return None;
    }
    std::fs::read_link(format!("/proc/{pid}/cwd"))
        .ok()?
        .into_os_string()
        .into_string()
        .ok()
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shutdown_pipe() -> (OwnedFd, OwnedFd) {
        let mut fds = [-1; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
    }

    #[test]
    fn input_backpressure_is_visible_atomic_and_recovers_in_order() {
        let (tx, rx) = mpsc::sync_channel(2);
        try_write(&tx, b"first").unwrap();
        try_write(&tx, b"second").unwrap();
        let rejected = b"a paste longer than inline storage";
        assert_eq!(try_write(&tx, rejected).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        assert_eq!(rx.recv().unwrap().as_slice(), b"first");
        try_write(&tx, rejected).unwrap();
        assert_eq!(rx.recv().unwrap().as_slice(), b"second");
        assert_eq!(rx.recv().unwrap().as_slice(), rejected);
        drop(rx);
        assert_eq!(try_write(&tx, b"closed").unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn protocol_reply_retries_stalled_queue_and_shutdown_cancels_wait() {
        let (shutdown, close) = shutdown_pipe();
        let (tx, rx) = mpsc::sync_channel(2);
        try_write(&tx, b"input1").unwrap();
        try_write(&tx, b"input2").unwrap();
        let (done_tx, done) = mpsc::channel();
        let producer = tx.clone();
        let worker = std::thread::spawn(move || {
            let result = queue_reply(&producer, WritePayload::from_slice(b"\x1b[1;1R"), shutdown.as_raw_fd());
            done_tx.send(result).unwrap();
        });
        assert!(done.recv_timeout(std::time::Duration::from_millis(20)).is_err());
        assert_eq!(rx.recv().unwrap().as_slice(), b"input1");
        assert_eq!(rx.recv().unwrap().as_slice(), b"input2");
        done.recv_timeout(std::time::Duration::from_secs(1)).unwrap().unwrap();
        assert_eq!(rx.recv().unwrap().as_slice(), b"\x1b[1;1R");
        worker.join().unwrap();
        drop(close);
        let (shutdown, close) = shutdown_pipe();
        let (tx, _rx) = mpsc::sync_channel(1);
        try_write(&tx, b"stalled").unwrap();
        let worker = std::thread::spawn(move || queue_reply(&tx, WritePayload::from_slice(b"reply"), shutdown.as_raw_fd()));
        drop(close);
        assert_eq!(worker.join().unwrap().unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn image_jobs_release_grid_lock_and_preserve_stream_order() {
        let grid = Arc::new(Mutex::new(Grid::new(80, 24)));
        let mut parser = vte::Parser::new();
        let mut reply = Vec::new();
        let mut jobs = 0;
        // The final chunk intentionally omits the placement keys. They must be
        // carried from the first chunk through decode and application.
        let stream = b"\x1b_Ga=T,i=7,f=32,s=1,v=1,c=3,r=2,C=1,m=1;AAAA\x1b\\\x1b_Gm=0;AA==\x1b\\\x1bP0;1q#1~\x1b\\Z";
        feed_output(&grid, &mut parser, stream, &mut reply, |job| {
            jobs += 1;
            let other = Arc::clone(&grid);
            std::thread::spawn(move || {
                let g = other.try_lock().expect("UI can acquire grid during image work");
                assert_ne!(g.display_row(0)[0].codepoint, b'Z' as u32, "following text must wait");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }).join().unwrap();
            job.decode()
        });
        assert_eq!(jobs, 2);
        let g = grid.lock();
        assert!(g.graphics.image(7).is_some());
        let placement = g.graphics.placements.iter().find(|p| p.image_id == 7).unwrap();
        assert_eq!((placement.cols, placement.rows), (3, 2));
        assert!(reply.windows(6).any(|b| b == b"i=7;OK"));
        assert!(g.display_row(1).iter().any(|c| c.codepoint == b'Z' as u32));
    }

    #[test]
    fn kitty_file_reads_are_deferred_and_ui_clear_cancels_inflight_image() {
        let grid = Arc::new(Mutex::new(Grid::new(80, 24)));
        let mut parser = vte::Parser::new();
        let mut reply = Vec::new();
        let path = std::env::temp_dir().join(format!("tty-graphics-protocol-deferred-{}-{}.raw",
            std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
        std::io::Write::write_all(&mut file, &[255, 0, 0, 255]).unwrap();
        drop(file);
        let mut encoded = String::new();
        const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        for bytes in path.to_str().unwrap().as_bytes().chunks(3) {
            let n = (bytes[0] as u32) << 16 | (*bytes.get(1).unwrap_or(&0) as u32) << 8 | *bytes.get(2).unwrap_or(&0) as u32;
            for i in 0..=bytes.len() { encoded.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char); }
        }
        let stream = format!("\x1b_Ga=T,t=t,i=8,f=32,s=1,v=1,C=1;{encoded}\x1b\\");
        feed_output(&grid, &mut parser, stream.as_bytes(), &mut reply, |job| {
            // Emulate a slow filesystem at the decode boundary. The UI can
            // still acquire its guard while the reader is stalled here.
            let other = Arc::clone(&grid);
            let ui = std::thread::spawn(move || {
                assert!(other.try_lock().is_some());
            });
            std::thread::sleep(std::time::Duration::from_millis(20));
            ui.join().unwrap();
            assert!(path.exists(), "temporary input has not been read under the grid lock");
            job.decode()
        });
        assert!(!path.exists(), "temporary medium is removed after reading");
        assert_eq!(&*grid.lock().graphics.image(8).unwrap().rgba, &[255, 0, 0, 255]);
        assert!(String::from_utf8_lossy(&reply).contains("i=8;OK"));
        reply.clear();
        // A denied path must still pass through the unlocked job, returning
        // exactly the same uniform protocol error as synchronous decoding.
        feed_output(&grid, &mut parser, b"\x1b_Ga=t,t=f,i=9,f=32,s=1,v=1;L2V0Yy9wYXNzd2Q=\x1b\\", &mut reply, |job| {
            assert!(grid.try_lock().is_some());
            job.decode()
        });
        assert!(String::from_utf8_lossy(&reply).contains("EPERM"));
        assert!(grid.lock().graphics.image(9).is_none());
        reply.clear();
        feed_output(&grid, &mut parser, b"\x1b_Ga=T,i=10,f=32,s=1,v=1;AAAAAA==\x1b\\", &mut reply, |job| {
            grid.lock().clear_screen_and_history();
            job.decode()
        });
        assert!(grid.lock().graphics.image(10).is_none());
        assert!(reply.is_empty());
    }

    #[test]
    fn dropping_pty_before_spawn_reader_reaps_the_shell() {
        let pty = Pty::spawn_in(80, 24, 8, 16, None, None).expect("spawn pty");
        let pid = pty.pid;
        drop(pty);
        let mut status = 0;
        let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        assert_eq!(r, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ECHILD));
    }

    #[test]
    fn dropping_pty_never_blocks_and_reaps_the_shell() {
        let pty = Pty::spawn_in(80, 24, 8, 16, None, None).expect("spawn pty");
        let pid = pty.pid;
        let (tx, rx) = std::sync::mpsc::channel();
        pty.spawn_reader(Arc::new(Mutex::new(Grid::new(80, 24))), || {}, move || {
            let _ = tx.send(());
        })
        .expect("spawn reader");

        // Far more than the kernel PTY buffer: must be queued, not block the caller.
        let payload = vec![b'x'; 4 << 20];
        let start = std::time::Instant::now();
        pty.write(&payload).unwrap();
        assert!(start.elapsed() < std::time::Duration::from_millis(200));

        drop(pty);
        rx.recv_timeout(std::time::Duration::from_secs(10)).expect("reader thread exits");
        // Reaped: the pid no longer exists as our child (not even as a zombie).
        let r = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
        assert_eq!(r, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ECHILD));
    }

    #[test]
    fn shell_exit_is_reported_and_reaped() {
        let pty = Pty::spawn_in(80, 24, 8, 16, None, None).expect("spawn pty");
        let pid = pty.pid;
        let (tx, rx) = std::sync::mpsc::channel();
        pty.spawn_reader(Arc::new(Mutex::new(Grid::new(80, 24))), || {}, move || {
            let _ = tx.send(());
        })
        .expect("spawn reader");
        pty.write(b"exit\r").unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(10)).expect("on_exit fires");
        let r = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
        assert_eq!(r, -1);
        drop(pty);
    }

    #[test]
    fn output_after_enter_is_not_held_back_waiting_for_the_prompt() {
        let pty = Pty::spawn_in(80, 24, 8, 16, None, None).expect("spawn pty");
        let grid = Arc::new(Mutex::new(Grid::new(80, 24)));
        let screen = |g: &Grid| -> String {
            let row = |r: usize| -> String { g.display_row(r).iter().map(|c| char::from_u32(c.codepoint).filter(|&c| c != '\0').unwrap_or(' ')).collect() };
            (0..24).map(|r| row(r).trim_end().to_string()).collect::<Vec<_>>().join("\n")
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let notify_grid = grid.clone();
        pty.spawn_reader(grid, move || {
            let _ = tx.send(screen(&notify_grid.lock()));
        }, || {})
        .expect("spawn reader");

        // Let the shell finish starting up before typing.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        while rx.try_recv().is_ok() {}
        // Like zsh after Enter: bracketed paste goes off, a newline is echoed, and the
        // prompt follows once its precmd finishes (10ms here; agnoster's git status
        // takes 40-200ms). `%s%s` keeps the markers out of the echoed command line.
        pty.write(b"printf '\\033[?2004h'; sleep 0.3; printf '\\033[?2004l\\r\\n%s%s' AF TER; sleep 0.01; printf '%s%s' PRO MPT\r").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut frames = Vec::new();
        while let Ok(text) = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            let done = text.contains("PROMPT");
            frames.push(text);
            if done {
                break;
            }
        }
        assert!(frames.last().is_some_and(|t| t.contains("PROMPT")), "prompt never arrived");
        assert!(
            frames.iter().any(|t| t.contains("AFTER") && !t.contains("PROMPT")),
            "the newline after Enter was only shown together with the prompt"
        );
    }

    #[test]
    fn shell_spawn_closes_inherited_fds() {
        let mut pipe_fds = [-1; 2];
        unsafe {
            assert_eq!(libc::pipe(pipe_fds.as_mut_ptr()), 0);
        }

        let pty = Pty::spawn_in(80, 24, 8, 16, None, None).expect("spawn pty");
        unsafe {
            // Close the write end in the parent process.
            libc::close(pipe_fds[1]);

            // If the spawned child process closed inherited file descriptors >= 3,
            // all write ends are closed, so read(pipe_fds[0]) must immediately return 0 (EOF)
            // without blocking, even while the spawned shell is still alive.
            let mut buf = [0u8; 1];
            let n = libc::read(pipe_fds[0], buf.as_mut_ptr() as _, 1);
            libc::close(pipe_fds[0]);
            assert_eq!(n, 0, "child process must have closed inherited write pipe end");
        }
        pty.write(b"exit\r").unwrap();
    }

    #[test]
    fn current_process_cwd_resolves() {
        let pid = unsafe { libc::getpid() };
        let cwd = pid_cwd(pid).expect("pid_cwd for current process");
        assert!(std::path::Path::new(&cwd).is_dir());
    }
}
