//! Private same-user activation sockets for `+reuse` on Linux; one blocking worker, asleep while idle.
use crate::platform::activation::{
    self as wire, ActivationAction, ActivationRequest, ActivationSink, Outcomes, TIMEOUT,
};
use std::{
    fs,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::{DirBuilderExt, FileTypeExt, MetadataExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_REGISTRY_ENTRIES: usize = 4096;
static PROCESS_STARTED_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(super) fn capture_start_time() {
    use std::sync::atomic::Ordering;
    match monotonic_ns() {
        Ok(started) => PROCESS_STARTED_NS.store(started, Ordering::Relaxed),
        Err(error) => {
            PROCESS_STARTED_NS.store(u64::MAX, Ordering::Relaxed);
            eprintln!("litty: activation start clock failed: {}", error.kind());
        }
    }
}

fn monotonic_ns() -> io::Result<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes one initialized timespec; monotonic is boot-relative.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return Err(io::Error::last_os_error());
    }
    u64::try_from(time.tv_sec)
        .ok()
        .and_then(|s| s.checked_mul(1_000_000_000))
        .and_then(|s| s.checked_add(time.tv_nsec as u64))
        .ok_or_else(wire::invalid)
}
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
fn uid() -> u32 {
    unsafe { libc::geteuid() }
}
fn hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(s: &str) -> io::Result<[u8; 16]> {
    if s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(wire::invalid());
    }
    let mut out = [0; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| wire::invalid())?;
    }
    Ok(out)
}
#[derive(Debug, Clone)]
struct Origin {
    name: String,
    started: u64,
    session: [u8; 16],
    pid: u32,
}
impl Origin {
    fn parse(name: &str) -> io::Result<Self> {
        let pieces: Vec<_> = name.split('-').collect();
        if pieces.len() != 3
            || pieces[0].len() != 16
            || pieces[1].is_empty()
            || pieces[1].len() > 8
            || !pieces[0]
                .bytes()
                .chain(pieces[1].bytes())
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err(wire::invalid());
        }
        let pid = u32::from_str_radix(pieces[1], 16).map_err(|_| wire::invalid())?;
        if pid == 0 || pid > i32::MAX as u32 {
            return Err(wire::invalid());
        }
        Ok(Self {
            name: name.into(),
            started: u64::from_str_radix(pieces[0], 16).map_err(|_| wire::invalid())?,
            session: unhex(pieces[2])?,
            pid,
        })
    }
    fn new() -> io::Result<Self> {
        let stamp = PROCESS_STARTED_NS.load(std::sync::atomic::Ordering::Relaxed);
        let started = match stamp {
            0 => monotonic_ns()?,
            u64::MAX => return Err(wire::invalid()),
            value => value,
        };
        Self::parse(&format!(
            "{:016x}-{:x}-{}",
            started,
            std::process::id(),
            hex(&nonce()?)
        ))
    }
}
fn private_dir(path: &Path, create: bool) -> io::Result<()> {
    if create {
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.uid() != uid() || meta.mode() & 0o777 != 0o700 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "unsafe activation directory",
        ));
    }
    Ok(())
}
/// Points tests and the reuse harness at a private registry so they never
/// discover or reach a person's running instances. It gets the same
/// owner/mode/symlink checks as the default per-user directory.
const ROOT_ENV: &str = "LITTY_ACTIVATION_DIR";
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
fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "activation deadline expired; request may have been delivered",
            )
        })
}
struct DeadlineStream {
    stream: UnixStream,
    deadline: Instant,
}
impl Read for DeadlineStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            wait_stream(&self.stream, libc::POLLIN, self.deadline)?;
            match self.stream.read(bytes) {
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                result => return result,
            }
        }
    }
}
impl Write for DeadlineStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        loop {
            wait_stream(&self.stream, libc::POLLOUT, self.deadline)?;
            match self.stream.write(bytes) {
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn wait_stream(stream: &UnixStream, events: i16, deadline: Instant) -> io::Result<()> {
    let mut fds = [libc::pollfd {
        fd: stream.as_raw_fd(),
        events,
        revents: 0,
    }];
    poll_ready(&mut fds, Some(deadline))
}
fn poll_ready(fds: &mut [libc::pollfd], deadline: Option<Instant>) -> io::Result<()> {
    loop {
        let timeout = match deadline {
            Some(end) => remaining(end)?.as_millis().max(1).min(i32::MAX as u128) as i32,
            None => -1,
        };
        // SAFETY: descriptors stay owned; the slice is valid throughout poll.
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
        if rc > 0 {
            if fds.iter().any(|p| p.revents & libc::POLLNVAL != 0) {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "activation socket closed",
                ));
            }
            return Ok(());
        }
        if rc == 0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "activation connection timed out",
            ));
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn connect(path: &Path) -> io::Result<DeadlineStream> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= address.sun_path.len() {
        return Err(wire::invalid());
    }
    address.sun_family = libc::AF_UNIX as _;
    for (dst, src) in address.sun_path.iter_mut().zip(bytes) {
        *dst = *src as _;
    }
    let deadline = Instant::now() + TIMEOUT;
    // SAFETY: no pointers; returns a newly owned descriptor on success.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    stream.set_nonblocking(true)?;
    let rc = unsafe {
        libc::connect(
            fd,
            (&address as *const libc::sockaddr_un).cast(),
            (std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1) as _,
        )
    };
    if rc != 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::EINPROGRESS) && e.kind() != io::ErrorKind::WouldBlock {
            return Err(e);
        }
        let mut fds = [libc::pollfd {
            fd,
            events: libc::POLLOUT,
            revents: 0,
        }];
        poll_ready(&mut fds, Some(deadline))?;
        if let Some(e) = stream.take_error()? {
            return Err(e);
        }
    }
    stream.set_nonblocking(true)?;
    peer(&stream)?;
    Ok(DeadlineStream { stream, deadline })
}
fn send(root: &Path, origin: &Origin, action: ActivationAction) -> io::Result<()> {
    private_dir(&root.join(&origin.name), false)?;
    let path = root.join(&origin.name).join("s");
    let meta = fs::symlink_metadata(&path)?;
    if !meta.file_type().is_socket() || meta.uid() != uid() {
        return Err(wire::invalid());
    }
    let mut stream =
        connect(&path).map_err(|e| io::Error::new(e.kind(), format!("activation connect: {e}")))?;
    let req = ActivationRequest {
        id: nonce()?,
        session: origin.session,
        expires_ns: monotonic_ns()?
            .checked_add(TIMEOUT.as_nanos() as u64)
            .ok_or_else(wire::invalid)?,
        action,
    };
    wire::write_frame(&mut stream, &req.encode()?)
        .map_err(|e| io::Error::new(e.kind(), format!("activation write: {e}")))?;
    match wire::read_frame(&mut stream)
        .map_err(|e| io::Error::new(e.kind(), format!("activation reply: {e}")))?
        .as_slice()
    {
        [0] => Ok(()),
        [1] => Err(io::Error::other("activation request rejected")),
        _ => Err(wire::invalid()),
    }
}
#[derive(Default)]
struct Admission {
    pending: Option<mpsc::Receiver<wire::ActivationReply>>,
}
impl Admission {
    fn ready(&mut self) -> bool {
        if self
            .pending
            .as_ref()
            .is_some_and(|reply| matches!(reply.try_recv(), Err(mpsc::TryRecvError::Empty)))
        {
            return false;
        }
        self.pending = None;
        true
    }
}

fn serve_one(
    stream: UnixStream,
    session: [u8; 16],
    sink: &ActivationSink,
    outcomes: &mut Outcomes,
    admission: &mut Admission,
) -> io::Result<()> {
    peer(&stream)?;
    stream.set_nonblocking(true)?;
    let mut stream = DeadlineStream {
        stream,
        deadline: Instant::now() + TIMEOUT,
    };
    let req = ActivationRequest::decode(&wire::read_frame(&mut stream)?)?;
    let result = (|| {
        if req.session != session {
            return Err(wire::invalid());
        }
        if let Some(reply) = outcomes.lookup(&req, monotonic_ns()?)? {
            return Ok(reply);
        }
        if !admission.ready() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "activation application queue is busy",
            ));
        }
        let (reply, rx) = mpsc::sync_channel(1);
        sink(req.clone(), reply);
        let timeout = match remaining(stream.deadline) {
            Ok(left) => left,
            Err(error) => {
                admission.pending = Some(rx);
                return Err(error);
            }
        };
        let result = match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                admission.pending = Some(rx);
                Err("activation acknowledgement timed out".into())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("activation application stopped".into())
            }
        };
        outcomes.record(req, result.clone());
        Ok(result)
    })();
    wire::write_frame(&mut stream, &[u8::from(!matches!(result, Ok(Ok(()))))])
}

pub struct ActivationServer {
    origin: Origin,
    root: PathBuf,
    stop: UnixStream,
    worker: Option<JoinHandle<()>>,
}
impl ActivationServer {
    pub fn origin(&self) -> &str {
        &self.origin.name
    }
    pub fn accepts(&self, request: &ActivationRequest) -> bool {
        request.session == self.origin.session
            && monotonic_ns().is_ok_and(|now| request.expires_ns > now)
    }
}
impl Drop for ActivationServer {
    fn drop(&mut self) {
        if let Err(e) = self.stop.write_all(&[1]) {
            eprintln!("litty: activation shutdown signal failed: {}", e.kind());
        }
        if self.worker.take().is_some_and(|w| w.join().is_err()) {
            eprintln!("litty: activation worker panicked");
        }
        let dir = self.root.join(&self.origin.name);
        if let Err(e) = fs::remove_file(dir.join("s")) {
            eprintln!("litty: activation socket cleanup failed: {}", e.kind());
        }
        if let Err(e) = fs::remove_dir(dir) {
            eprintln!("litty: activation directory cleanup failed: {}", e.kind());
        }
    }
}
pub fn start_activation(sink: ActivationSink) -> io::Result<ActivationServer> {
    start_at(root(true)?, sink)
}
fn start_at(root: PathBuf, sink: ActivationSink) -> io::Result<ActivationServer> {
    // Tombstones of crashed or killed instances are otherwise reaped only by a
    // later `--reuse` scan; advertising is a cheap, bounded point to clean up.
    if let Err(e) = discover(&root, process_alive) {
        eprintln!("litty: activation registry cleanup failed: {}", e.kind());
    }
    start_with_origin(root, sink, Origin::new()?)
}
fn start_with_origin(
    root: PathBuf,
    sink: ActivationSink,
    origin: Origin,
) -> io::Result<ActivationServer> {
    let directory = root.join(&origin.name);
    private_dir(&directory, true)?;
    let setup = (|| {
        let listener = UnixListener::bind(directory.join("s"))?;
        listener.set_nonblocking(true)?;
        let (stop, wake) = UnixStream::pair()?;
        let session = origin.session;
        let worker = thread::Builder::new()
            .name("activation".into())
            .stack_size(128 * 1024)
            .spawn(move || {
                let mut outcomes = Outcomes::default();
                let mut admission = Admission::default();
                loop {
                    let mut fds = [
                        libc::pollfd {
                            fd: listener.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        },
                        libc::pollfd {
                            fd: wake.as_raw_fd(),
                            events: libc::POLLIN,
                            revents: 0,
                        },
                    ];
                    if let Err(e) = poll_ready(&mut fds, None) {
                        eprintln!("litty: activation listener failed: {}", e.kind());
                        break;
                    }
                    if fds[1].revents != 0 {
                        break;
                    }
                    if fds[0].revents & libc::POLLIN == 0 {
                        eprintln!("litty: activation listener closed");
                        break;
                    }
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if let Err(e) =
                                serve_one(stream, session, &sink, &mut outcomes, &mut admission)
                            {
                                eprintln!("litty: activation request failed: {}", e.kind());
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                        Err(e) => {
                            eprintln!("litty: activation accept failed: {}", e.kind());
                            break;
                        }
                    }
                }
            })?;
        Ok((stop, worker))
    })();
    match setup {
        Ok((stop, worker)) => Ok(ActivationServer {
            origin,
            root,
            stop,
            worker: Some(worker),
        }),
        Err(e) => {
            match fs::remove_file(directory.join("s")) {
                Ok(()) => {}
                Err(clean) if clean.kind() == io::ErrorKind::NotFound => {}
                Err(clean) => eprintln!("litty: activation setup cleanup failed: {}", clean.kind()),
            }
            if let Err(clean) = fs::remove_dir(directory) {
                eprintln!("litty: activation setup cleanup failed: {}", clean.kind());
            }
            Err(e)
        }
    }
}
fn process_alive(pid: u32) -> bool {
    // ESRCH is the only proof that a process is gone; EPERM means preserve it.
    (unsafe { libc::kill(pid as libc::pid_t, 0) == 0 })
        || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}
fn remove_if_present(path: &Path, remove: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    match remove(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn discover(root: &Path, alive: impl Fn(u32) -> bool) -> io::Result<Vec<Origin>> {
    let mut origins = Vec::new();
    for (count, entry) in fs::read_dir(root)?.enumerate() {
        if count >= MAX_REGISTRY_ENTRIES {
            return Err(io::Error::other("activation registry scan limit reached"));
        }
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(origin) = Origin::parse(&name) else {
            continue;
        };
        if !alive(origin.pid) {
            let directory = root.join(&origin.name);
            match private_dir(&directory, false) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            }
            let socket = directory.join("s");
            match fs::symlink_metadata(&socket) {
                Ok(metadata) if metadata.file_type().is_socket() && metadata.uid() == uid() => {
                    remove_if_present(&socket, |path| fs::remove_file(path))?
                }
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "unsafe activation tombstone",
                    ));
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            // remove_dir refuses nonempty directories: never delete unknown data.
            remove_if_present(&directory, |path| fs::remove_dir(path))?;
            continue;
        }
        origins.push(origin);
    }
    Ok(origins)
}

fn reuse_at(root: &Path, cwd: &str) -> io::Result<bool> {
    wire::validate_cwd(cwd)?;
    let mut origins = discover(root, process_alive)?;
    origins.sort_by(|a, b| b.started.cmp(&a.started).then(b.name.cmp(&a.name)));
    for origin in origins {
        match send(root, &origin, ActivationAction::NewWindow(cwd.into())) {
            Ok(()) => return Ok(true),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(false)
}
pub fn reuse_window(cwd: &str) -> io::Result<bool> {
    wire::validate_cwd(cwd)?;
    reuse_from(root(false), cwd)
}
fn reuse_from(dir: io::Result<PathBuf>, cwd: &str) -> io::Result<bool> {
    match dir {
        Ok(dir) => reuse_at(&dir, cwd),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        // Another user may pre-create the fixed /tmp name. No instance of ours
        // can be listening there, so start a new one instead of failing.
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            eprintln!("litty: ignoring unsafe activation directory; starting a new instance");
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

pub struct NotificationRouter;
pub fn start_notification_router() -> std::io::Result<NotificationRouter> {
    Ok(NotificationRouter)
}
pub fn notify_pane(_: Option<&str>, _: u64, title: &str, body: &str) {
    crate::platform::process::show_notification(title, body);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = PathBuf::from(format!("/tmp/la-test-{}", hex(&nonce().unwrap())));
            private_dir(&p, true).unwrap();
            Self(p)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            // A second panic while a failed test unwinds would abort the run
            // and hide the original assertion.
            if let Err(e) = fs::remove_dir(&self.0)
                && !thread::panicking()
            {
                panic!("fixture {} not empty or missing: {e}", self.0.display());
            }
        }
    }
    #[test]
    fn timed_out_request_keeps_admission_until_app_consumes_it() {
        let (reply, pending) = mpsc::sync_channel(1);
        let mut admission = Admission {
            pending: Some(pending),
        };
        assert!(
            !admission.ready(),
            "timeout must not admit another queued app event"
        );
        reply.send(Ok(())).unwrap();
        assert!(admission.ready());
        assert!(admission.pending.is_none());
    }

    #[test]
    fn real_transport_selects_latest_and_cleans_up() {
        let f = Fixture::new();
        assert!(!reuse_at(&f.0, "/tmp").unwrap());
        let (tx, rx) = mpsc::sync_channel(4);
        let first = tx.clone();
        let a = start_with_origin(
            f.0.clone(),
            Arc::new(move |req, reply| {
                first.send((1, req.action)).unwrap();
                reply.send(Ok(())).unwrap();
            }),
            Origin::parse(&format!("{:016x}-1-{}", 10, hex(&[1; 16]))).unwrap(),
        )
        .unwrap();
        let b = start_with_origin(
            f.0.clone(),
            Arc::new(move |req, reply| {
                tx.send((2, req.action)).unwrap();
                reply.send(Ok(())).unwrap();
            }),
            Origin::parse(&format!("{:016x}-1-{}", 20, hex(&[2; 16]))).unwrap(),
        )
        .unwrap();
        assert!(b.origin.started >= a.origin.started);
        assert!(reuse_at(&f.0, "/tmp/tiếng việt").unwrap());
        assert_eq!(
            rx.recv_timeout(TIMEOUT).unwrap(),
            (2, ActivationAction::NewWindow("/tmp/tiếng việt".into()))
        );
        drop(b);
        assert!(reuse_at(&f.0, "/tmp").unwrap());
        assert_eq!(rx.recv_timeout(TIMEOUT).unwrap().0, 1);
        drop(a);
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 0);
    }
    #[test]
    fn concurrent_tombstone_removal_is_idempotent() {
        let fixture = Fixture::new();
        let socket = fixture.0.join("s");
        fs::write(&socket, b"test-owned placeholder").unwrap();
        let file_result = remove_if_present(&socket, |path| {
            fs::remove_file(path)?; // The other pruner wins immediately before us.
            fs::remove_file(path)
        });
        let directory = fixture.0.join("dead");
        private_dir(&directory, true).unwrap();
        let directory_result = remove_if_present(&directory, |path| {
            fs::remove_dir(path)?;
            fs::remove_dir(path)
        });
        assert!(
            file_result.is_ok(),
            "a concurrent socket deletion is already complete"
        );
        assert!(
            directory_result.is_ok(),
            "a concurrent directory deletion is already complete"
        );
        let denied = remove_if_present(&socket, |_| {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        });
        assert_eq!(
            denied.unwrap_err().kind(),
            io::ErrorKind::PermissionDenied,
            "other failures must be preserved"
        );
    }

    #[test]
    fn crash_tombstones_do_not_saturate_instance_discovery() {
        let fixture = Fixture::new();
        let mut paths = Vec::new();
        for i in 0..65u8 {
            let origin =
                Origin::parse(&format!("{:016x}-1-{}", u64::from(i), hex(&[i; 16]))).unwrap();
            let path = fixture.0.join(origin.name);
            private_dir(&path, true).unwrap();
            paths.push(path);
        }
        let result = discover(&fixture.0, |_| false);
        let remaining = fs::read_dir(&fixture.0).unwrap().count();
        for path in paths {
            if path.exists() {
                fs::remove_dir(path).unwrap();
            }
        }
        assert!(result.is_ok(), "crash history must not prevent discovery");
        assert!(result.unwrap().is_empty());
        assert_eq!(
            remaining, 0,
            "only verified-dead private endpoints should be removed"
        );
    }

    #[test]
    fn notification_targets_its_origin_instead_of_newest_instance() {
        let fixture = Fixture::new();
        let (events, received) = mpsc::sync_channel(4);
        let old_events = events.clone();
        let old = start_with_origin(
            fixture.0.clone(),
            Arc::new(move |request, reply| {
                old_events.send((1, request.action)).unwrap();
                reply.send(Ok(())).unwrap();
            }),
            Origin::parse(&format!("{:016x}-1-{}", 10, hex(&[1; 16]))).unwrap(),
        )
        .unwrap();
        let newest = start_with_origin(
            fixture.0.clone(),
            Arc::new(move |request, reply| {
                events.send((2, request.action)).unwrap();
                reply.send(Ok(())).unwrap();
            }),
            Origin::parse(&format!("{:016x}-1-{}", 20, hex(&[2; 16]))).unwrap(),
        )
        .unwrap();
        send(&fixture.0, &old.origin, ActivationAction::Pane(42)).unwrap();
        assert_eq!(
            received.recv_timeout(TIMEOUT).unwrap(),
            (1, ActivationAction::Pane(42))
        );
        let stale = old.origin.clone();
        drop(old);
        assert!(send(&fixture.0, &stale, ActivationAction::Pane(42)).is_err());
        assert!(
            received.try_recv().is_err(),
            "stale origin must never target another instance"
        );
        drop(newest);
    }

    #[test]
    fn real_requests_reject_stale_sessions_and_do_not_replay_actions() {
        let fixture = Fixture::new();
        let (sent, accepted) = mpsc::sync_channel(4);
        let server = start_at(
            fixture.0.clone(),
            Arc::new(move |request, reply| {
                sent.send(request.action).unwrap();
                reply.send(Ok(())).unwrap();
            }),
        )
        .unwrap();
        let request = ActivationRequest {
            id: [7; 16],
            session: server.origin.session,
            expires_ns: monotonic_ns().unwrap() + TIMEOUT.as_nanos() as u64,
            action: ActivationAction::Pane(42),
        };
        let exchange = |request: &ActivationRequest| {
            let mut connection = connect(&fixture.0.join(&server.origin.name).join("s")).unwrap();
            wire::write_frame(&mut connection, &request.encode().unwrap()).unwrap();
            wire::read_frame(&mut connection).unwrap()
        };
        assert_eq!(exchange(&request), [0]);
        assert_eq!(
            accepted.recv_timeout(TIMEOUT).unwrap(),
            ActivationAction::Pane(42)
        );
        assert_eq!(exchange(&request), [0]);
        assert!(
            accepted.try_recv().is_err(),
            "replayed request must not dispatch another action"
        );
        let mut stale = request.clone();
        stale.id = [8; 16];
        stale.session = [0; 16];
        assert_eq!(exchange(&stale), [1]);
        let mut expired = request;
        expired.id = [9; 16];
        expired.expires_ns = 1;
        assert_eq!(exchange(&expired), [1]);
        assert!(
            accepted.try_recv().is_err(),
            "invalid identities must never reach app state"
        );
    }

    #[test]
    fn origin_parser_rejects_paths_and_bad_session_identifiers() {
        for s in ["../../tmp", "", "0-1-aa", "0000000000000001-1-🙂"] {
            assert!(Origin::parse(s).is_err());
        }
        let origin = Origin::new().unwrap();
        assert_eq!(Origin::parse(&origin.name).unwrap().session, origin.session);
    }
    #[test]
    fn unsafe_directories_and_symlinks_are_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let f = Fixture::new();
        let link = f.0.join("link");
        symlink(&f.0, &link).unwrap();
        assert!(private_dir(&link, false).is_err());
        fs::remove_file(link).unwrap();
        fs::set_permissions(&f.0, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            private_dir(&f.0, false).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }
    #[test]
    fn rejected_delivery_is_an_error_instead_of_fallback() {
        let f = Fixture::new();
        let server = start_at(
            f.0.clone(),
            Arc::new(|_, reply| reply.send(Err("failed".into())).unwrap()),
        )
        .unwrap();
        assert!(reuse_at(&f.0, "/tmp").is_err());
        drop(server);
    }
    #[test]
    fn registry_override_must_be_absolute_and_private() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        assert_eq!(root_at(Some(f.0.clone().into()), None, false).unwrap(), f.0);
        assert_eq!(
            root_at(Some("relative/dir".into()), None, false).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        fs::set_permissions(&f.0, fs::Permissions::from_mode(0o755)).unwrap();
        let unsafe_root = root_at(Some(f.0.clone().into()), None, false);
        assert_eq!(unsafe_root.as_ref().unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        // A root another user controls cannot hold our instances: start fresh.
        assert!(!reuse_from(unsafe_root, "/tmp").unwrap());
        fs::set_permissions(&f.0, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            reuse_from(Err(io::Error::other("broken")), "/tmp").unwrap_err().kind(),
            io::ErrorKind::Other
        );
    }
    #[test]
    fn advertising_reaps_dead_instance_tombstones() {
        let f = Fixture::new();
        // PIDs above the kernel's maximum never exist (kill reports ESRCH).
        let dead = Origin::parse(&format!("{:016x}-7ffffff0-{}", 1, hex(&[9; 16]))).unwrap();
        let tombstone = f.0.join(&dead.name);
        private_dir(&tombstone, true).unwrap();
        let server = start_at(f.0.clone(), Arc::new(|_, reply| reply.send(Ok(())).unwrap())).unwrap();
        assert!(!tombstone.exists(), "dead instance tombstone survived advertising");
        assert!(f.0.join(server.origin()).join("s").exists());
        drop(server);
    }
    #[test]
    fn late_acknowledgement_is_an_error_without_fallback_or_second_admission() {
        let f = Fixture::new();
        let (older_tx, older_rx) = mpsc::sync_channel(4);
        let older = start_with_origin(
            f.0.clone(),
            Arc::new(move |req, reply| {
                older_tx.send(req.action).unwrap();
                reply.send(Ok(())).unwrap();
            }),
            Origin::parse(&format!("{:016x}-1-{}", 10, hex(&[1; 16]))).unwrap(),
        )
        .unwrap();
        // The newest instance's app holds its first acknowledgement past the
        // deadline (it was delivered, so the caller must not try elsewhere).
        let (held_tx, held_rx) = mpsc::sync_channel(1);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = calls.clone();
        let newest = start_with_origin(
            f.0.clone(),
            Arc::new(move |_, reply| {
                if seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    held_tx.send(reply).unwrap();
                } else {
                    reply.send(Ok(())).unwrap();
                }
            }),
            Origin::parse(&format!("{:016x}-1-{}", 20, hex(&[2; 16]))).unwrap(),
        )
        .unwrap();

        assert!(reuse_at(&f.0, "/tmp").is_err(), "late acknowledgement must be an error");
        let held = held_rx.recv_timeout(TIMEOUT).expect("request reached the newest app");
        assert!(reuse_at(&f.0, "/tmp").is_err(), "busy app must not admit a second request");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(older_rx.try_recv().is_err(), "no fallback to an older instance");

        held.send(Ok(())).unwrap();
        assert!(reuse_at(&f.0, "/tmp").unwrap(), "consumed acknowledgement frees admission");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(older_rx.try_recv().is_err());
        drop(newest);
        drop(older);
    }
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
}
