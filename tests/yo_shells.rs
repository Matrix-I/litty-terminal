//! Isolated PTY smoke tests for `yo` interception on zsh, Bash, fish, and nushell.
//!
//! Each test drives a real interactive shell with the Litty integration script
//! loaded. Run outside a PTY-restricted sandbox; no API keys or network
//! requests are used. A shell that is not installed is skipped.
#![cfg(unix)]

use std::{
    fs::{self, File},
    io::{Error, ErrorKind, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{fs::PermissionsExt, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

const READ_WINDOW: Duration = Duration::from_millis(400);
/// Nushell writes a default config on first start, so its first read waits longer.
const NU_STARTUP_WINDOW: Duration = Duration::from_secs(4);
/// Reply to a cursor-position query (`ESC [ 6 n`) as a real terminal would.
const CURSOR_QUERY: &[u8] = b"\x1b[6n";
const CURSOR_REPLY: &[u8] = b"\x1b[1;1R";

/// An interactive shell attached to a fresh PTY, killed and cleaned up on drop.
struct Session {
    child: Child,
    master: File,
    home: PathBuf,
}

impl Session {
    fn spawn(program: &Path, args: &[String], home: &Path, extra_env: &[(&str, &Path)]) -> Self {
        let (mut master, mut slave) = (-1, -1);
        let mut size = libc::winsize { ws_row: 24, ws_col: 120, ws_xpixel: 0, ws_ypixel: 0 };
        // SAFETY: both out-pointers are valid; termios is optional (null). `winp` is
        // `*mut` on macOS but `*const` on Linux, so pass a raw pointer to fit both.
        let rc = unsafe {
            libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &raw mut size)
        };
        assert_eq!(rc, 0, "openpty: {}", Error::last_os_error());
        // SAFETY: openpty returned two fresh descriptors that nothing else owns.
        let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        // SAFETY: plain fcntl on a descriptor we own; keeps the master out of the child.
        unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };

        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(home)
            .env("HOME", home)
            .env("ZDOTDIR", home)
            .env("TERM", "xterm-256color")
            .env("PS1", "test> ")
            .env("HISTFILE", "/dev/null")
            .envs(extra_env.iter().copied())
            .stdin(Stdio::from(slave.try_clone().expect("dup slave")))
            .stdout(Stdio::from(slave.try_clone().expect("dup slave")))
            .stderr(Stdio::from(slave));
        // SAFETY: only async-signal-safe calls (setsid, ioctl) run after fork.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().expect("spawn shell");
        // Close the parent's copies of the slave so the master sees hangup.
        drop(command);
        Self { child, master: File::from(master), home: home.to_path_buf() }
    }

    fn write(&mut self, data: &[u8]) {
        self.master.write_all(data).expect("write to pty");
    }

    fn read(&mut self) -> Vec<u8> {
        self.read_for(READ_WINDOW)
    }

    /// Collects output until the window elapses, mirroring a timed `select` loop.
    /// Cursor-position queries are answered so line editors that wait on them proceed.
    fn read_for(&mut self, window: Duration) -> Vec<u8> {
        let end = Instant::now() + window;
        let mut data = Vec::new();
        let mut buf = [0u8; 65536];
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            let mut pfd = libc::pollfd { fd: self.master.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            let timeout = i32::try_from(left.as_millis()).unwrap_or(i32::MAX);
            // SAFETY: `pfd` is a valid pollfd and nfds is 1.
            let ready = unsafe { libc::poll(&mut pfd, 1, timeout) };
            if ready < 0 && Error::last_os_error().kind() == ErrorKind::Interrupted {
                continue;
            }
            if ready <= 0 {
                break;
            }
            match self.master.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    data.extend_from_slice(&buf[..n]);
                    for _ in buf[..n].windows(CURSOR_QUERY.len()).filter(|w| *w == CURSOR_QUERY) {
                        self.write(CURSOR_REPLY);
                    }
                }
            }
        }
        data
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.home);
    }
}

fn count(data: &[u8], needle: &[u8]) -> usize {
    data.windows(needle.len()).filter(|window| *window == needle).count()
}

fn has(data: &[u8], needle: &[u8]) -> bool {
    data.windows(needle.len()).any(|window| window == needle)
}

fn text(data: &[u8]) -> String {
    String::from_utf8_lossy(data).into_owned()
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| {
            fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn base64_decode(input: &[u8]) -> Vec<u8> {
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0u32);
    for &byte in input {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\r' | b'\n' => continue,
            other => panic!("invalid base64 byte {other:#x}"),
        };
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

/// Starts `shell` with the Litty integration loaded and `env` added to its
/// environment; `None` when the shell is not installed. Also returns what the
/// shell printed during startup.
fn launch(shell: &str, env: &[(&str, &Path)]) -> Option<(Session, Vec<u8>)> {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let Some(binary) = which(shell) else {
        eprintln!("{shell}: skipped (not installed)");
        return None;
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let home = std::env::temp_dir().join(format!("litty-yo-{shell}-{}-{unique}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).expect("create home");

    let data_dirs = home.join("data");
    let config_home = home.join(".config");
    let mut extra_env: Vec<(&str, &Path)> = env.to_vec();
    let args: Vec<String> = match shell {
        "zsh" => {
            let script = shell_quote(&root.join("src/shell/litty-integration.zsh"));
            fs::write(home.join(".zshrc"), format!("PROMPT='test> '\nsource {script}\n")).expect("write .zshrc");
            vec!["-i".into()]
        }
        "fish" => {
            let config = home.join(".config/fish/config.fish");
            fs::create_dir_all(config.parent().expect("config dir")).expect("create fish config dir");
            let script = shell_quote(&root.join("src/shell/litty-integration.fish"));
            fs::write(&config, format!("source {script}\n")).expect("write config.fish");
            vec!["--interactive".into()]
        }
        "nu" => {
            // Mirror Litty's injection: the script is autoloaded from XDG_DATA_DIRS.
            let autoload = data_dirs.join("nushell/vendor/autoload");
            fs::create_dir_all(&autoload).expect("create nushell autoload dir");
            fs::copy(root.join("src/shell/litty.nu"), autoload.join("litty.nu")).expect("copy litty.nu");
            extra_env.push(("XDG_DATA_DIRS", &data_dirs));
            extra_env.push(("XDG_CONFIG_HOME", &config_home));
            Vec::new()
        }
        _ => vec![
            "--noprofile".into(),
            "--rcfile".into(),
            root.join("src/shell/litty.bash").to_string_lossy().into_owned(),
            "-i".into(),
        ],
    };

    let mut session = Session::spawn(&binary, &args, &home, &extra_env);
    let initial = session.read_for(if shell == "nu" { NU_STARTUP_WINDOW } else { READ_WINDOW });
    assert!(has(&initial, b"778;init"), "{shell}: {}", text(&initial));
    Some((session, initial))
}

fn run(shell: &str) {
    let Some((mut session, initial)) = launch(shell, &[]) else { return };

    let query = "find files > 100MB; echo SHOULD_NOT_EXECUTE";
    let direct = has(&initial, b"778;init;direct");
    let output = if direct {
        session.write(format!("yo {query}").as_bytes());
        session.read();
        // Simulate Litty's direct interception before it forwards Enter.
        session.write(b"\x05\x15\r");
        let output = session.read();
        assert!(!has(&output, b"SHOULD_NOT_EXECUTE\r\n"), "{shell}: {}", text(&output));
        output
    } else {
        session.write(format!("yo {query}\r").as_bytes());
        let output = session.read();
        let marker: &[u8] = b"\x1b]778;yo;";
        let start = output
            .windows(marker.len())
            .position(|window| window == marker)
            .unwrap_or_else(|| panic!("{shell}: {}", text(&output)));
        let rest = &output[start + marker.len()..];
        let payload = rest.split(|&byte| byte == 0x07).next().unwrap_or(rest);
        let decoded = base64_decode(payload);
        assert_eq!(text(&decoded).trim(), query, "{shell}: {}", text(&output));
        // A second prompt-ready mark makes Litty draw `Thinking...` on a prompt the
        // shell is about to replace, leaving duplicate prompts on screen.
        assert_eq!(count(rest, b"133;B"), 1, "{shell}: {}", text(&output));
        if shell == "zsh" {
            // The empty prompt zsh redraws for the abandoned `yo` line is erased
            // (one row for this test's single-line prompt) before the real one.
            assert!(has(rest, b"\x1b[1A\x1b[J"), "{shell}: {}", text(&output));
        }
        output
    };
    assert!(!has(&output, b"command not found"), "{shell}: {}", text(&output));

    // The private repaint chord must emit prompt-ready, not insert literal escape text.
    session.write(b"\x1b[24;8~");
    let repaint = session.read();
    assert!(has(&repaint, b"133;B"), "{shell}: {}", text(&repaint));

    // Prefill alone cannot execute. An explicit Enter executes after edits.
    if has(&initial, b"?2004h") {
        session.write(b"\x1b[200~printf AI_TEST\x1b[201~");
    } else {
        session.write(b"printf AI_TEST");
    }
    let prefill = session.read();
    assert!(!has(&prefill, b"133;C"), "{shell}: {}", text(&prefill));
    session.write(b"_EDITED\r");
    let executed = session.read();
    assert!(has(&executed, b"AI_TEST_EDITED") && has(&executed, b"133;C"), "{shell}: {}", text(&executed));
    session.write(b"printf NORMAL_TEST\r");
    let normal = session.read();
    assert!(has(&normal, b"NORMAL_TEST") && has(&normal, b"133;C"), "{shell}: {}", text(&normal));
}

#[test]
fn yo_zsh() {
    run("zsh");
}

#[test]
fn yo_bash() {
    run("bash");
}

#[test]
fn yo_fish() {
    run("fish");
}

#[test]
fn yo_nu() {
    run("nu");
}

/// A directory with an executable `yo` that reports its arguments, like a real
/// Yeoman install; removed on drop.
struct StubYo(PathBuf);

impl StubYo {
    fn new(shell: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("litty-yo-stub-{shell}-{}-{unique}", std::process::id()));
        fs::create_dir_all(&dir).expect("create stub dir");
        let stub = dir.join("yo");
        fs::write(&stub, "#!/bin/sh\necho \"STUB_YO_RECEIVED $*\"\n").expect("write stub");
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).expect("chmod stub");
        Self(dir)
    }

    fn path_with_stub_first(&self) -> PathBuf {
        let mut dirs = vec![self.0.clone()];
        dirs.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        std::env::join_paths(dirs).expect("join PATH").into()
    }
}

impl Drop for StubYo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// With another trigger or none, `yo` is an ordinary command again: a stub
/// `yo` on PATH receives `yo generator`, and nothing is sent to the AI bridge.
fn run_without_yo_trigger(shell: &str, trigger: &str) {
    let stub = StubYo::new(shell);
    let path = stub.path_with_stub_first();
    let env = [("LITTY_AI_TRIGGER", Path::new(trigger)), ("PATH", path.as_path())];
    let Some((mut session, initial)) = launch(shell, &env) else { return };

    session.write(b"yo generator\r");
    let output = session.read();
    assert!(has(&output, b"STUB_YO_RECEIVED generator"), "{shell}/{trigger:?}: {}", text(&output));
    assert!(!has(&output, b"778;yo;"), "{shell}/{trigger:?}: {}", text(&output));

    // Shells that edit the line themselves also honour the renamed word.
    if !trigger.is_empty() && !has(&initial, b"778;init;direct") {
        session.write(format!("{trigger} list files\r").as_bytes());
        let output = session.read();
        let marker: &[u8] = b"\x1b]778;yo;";
        let start = output
            .windows(marker.len())
            .position(|window| window == marker)
            .unwrap_or_else(|| panic!("{shell}/{trigger:?}: {}", text(&output)));
        let rest = &output[start + marker.len()..];
        let payload = rest.split(|&byte| byte == 0x07).next().unwrap_or(rest);
        assert_eq!(text(&base64_decode(payload)).trim(), "list files", "{shell}: {}", text(&output));
    }
}

#[test]
fn renamed_trigger_zsh() {
    run_without_yo_trigger("zsh", "ask");
}

#[test]
fn renamed_trigger_bash() {
    run_without_yo_trigger("bash", "ask");
}

#[test]
fn renamed_trigger_fish() {
    run_without_yo_trigger("fish", "ask");
}

#[test]
fn renamed_trigger_nu() {
    run_without_yo_trigger("nu", "ask");
}

#[test]
fn disabled_trigger_zsh() {
    run_without_yo_trigger("zsh", "");
}

#[test]
fn disabled_trigger_bash() {
    run_without_yo_trigger("bash", "");
}

#[test]
fn disabled_trigger_fish() {
    run_without_yo_trigger("fish", "");
}

#[test]
fn disabled_trigger_nu() {
    run_without_yo_trigger("nu", "");
}
