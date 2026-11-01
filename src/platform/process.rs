//! Detached helper processes: `open` / `xdg-open`, notifications, sounds.
//!
//! URLs and paths are always passed as a single argument, never through a shell.

/// Resolve system utility to an absolute path when available to prevent PATH hijacking.
pub(crate) fn system_bin(name: &'static str) -> &'static str {
    match name {
        "open" => "/usr/bin/open",
        "osascript" => "/usr/bin/osascript",
        "afplay" => "/usr/bin/afplay",
        "xdg-open" => if std::path::Path::new("/usr/bin/xdg-open").exists() { "/usr/bin/xdg-open" } else { "xdg-open" },
        "notify-send" => if std::path::Path::new("/usr/bin/notify-send").exists() { "/usr/bin/notify-send" } else { "notify-send" },
        "paplay" => if std::path::Path::new("/usr/bin/paplay").exists() { "/usr/bin/paplay" } else { "paplay" },
        "zenity" => if std::path::Path::new("/usr/bin/zenity").exists() { "/usr/bin/zenity" } else { "zenity" },
        _ => name,
    }
}

/// Play a sound file in the background (`afplay` on macOS, PulseAudio's
/// `paplay` elsewhere); the path is a single argument, never parsed by a shell.
pub(crate) fn play_sound(path: &std::path::Path, volume: f32) {
    #[cfg(target_os = "macos")]
    let res = spawn_detached(
        std::process::Command::new("/usr/bin/afplay")
            .arg("-v")
            .arg(volume.to_string())
            .arg("--")
            .arg(path),
    );
    #[cfg(not(target_os = "macos"))]
    let res = spawn_detached(
        std::process::Command::new(system_bin("paplay"))
            .arg(format!("--volume={}", (volume * 65536.0) as u32))
            .arg("--")
            .arg(path),
    );
    if let Err(e) = res {
        eprintln!("litty: cannot play {}: {e}", path.display());
    }
}

/// Show a desktop notification. Text from the terminal is passed as separate
/// arguments (never through a shell), with leading `-` stripped so it cannot be
/// read as an option.
pub(crate) fn show_notification(title: &str, body: &str) {
    let title = title.trim_start_matches('-');
    let body = body.trim_start_matches('-');
    #[cfg(target_os = "macos")]
    let res = spawn_detached(
        std::process::Command::new(system_bin("osascript"))
            .args(["-e", "on run argv", "-e", "display notification (item 2 of argv) with title (item 1 of argv)"])
            .args(["-e", "end run"])
            .arg(title)
            .arg(body),
    );
    #[cfg(not(target_os = "macos"))]
    let res = spawn_detached(std::process::Command::new(system_bin("notify-send")).arg("--").arg(title).arg(body));
    if let Err(e) = res {
        eprintln!("litty: failed to show notification: {e}");
    }
}

static IN_FLIGHT_DETACHED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
const MAX_CONCURRENT_DETACHED: usize = 16;

/// Spawn a helper process (`open`, `xdg-open`) and reap it on a background thread
/// so it never lingers as a zombie, with bounded concurrency to prevent fork bombs.
pub(crate) fn spawn_detached(cmd: &mut std::process::Command) -> std::io::Result<()> {
    if IN_FLIGHT_DETACHED.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= MAX_CONCURRENT_DETACHED {
        IN_FLIGHT_DETACHED.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        return Err(std::io::Error::new(std::io::ErrorKind::WouldBlock, "too many concurrent detached processes"));
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            IN_FLIGHT_DETACHED.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            return Err(e);
        }
    };
    let pid = child.id();
    if let Err(e) = std::thread::Builder::new()
        .name("reap-helper".into())
        .stack_size(32 * 1024)
        .spawn(move || {
            let _ = child.wait();
            IN_FLIGHT_DETACHED.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        })
    {
        IN_FLIGHT_DETACHED.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        #[cfg(unix)]
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
            let mut status = 0;
            libc::waitpid(pid as libc::pid_t, &mut status, 0);
        }
        return Err(e);
    }
    Ok(())
}

#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn safe_path_for_linux_open(path: &std::path::Path) -> &std::path::Path {
    use std::os::unix::fs::PermissionsExt;
    let is_exec = path
        .metadata()
        .map(|m| m.is_file() && (m.permissions().mode() & 0o111 != 0))
        .unwrap_or(false);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let is_script_ext = matches!(
        ext.to_ascii_lowercase().as_str(),
        "sh" | "bash" | "zsh" | "bin" | "run" | "desktop" | "elf" | "py" | "pl" | "command" | "appimage"
    );
    if is_exec || is_script_ext {
        path.parent().unwrap_or(path)
    } else {
        path
    }
}

/// xdg-open accepts one target and no general `--` separator. Prefix relative
/// hyphen-leading paths so the target cannot be parsed as an option.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn linux_open_command(target: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    use std::os::unix::ffi::OsStrExt;
    let target = target.as_ref();
    let mut cmd = std::process::Command::new(system_bin("xdg-open"));
    if target.as_bytes().first() == Some(&b'-') {
        cmd.arg(std::path::Path::new(".").join(target));
    } else {
        cmd.arg(target);
    }
    cmd
}

pub(crate) fn open_target(target: &str, is_path: bool, cwd: Option<&std::path::Path>) {
    if is_path {
        let path = if let Some(rest) = target.strip_prefix("~/") {
            match std::env::var("HOME") {
                Ok(home) => std::path::PathBuf::from(home).join(rest),
                Err(_) => std::path::PathBuf::from(target),
            }
        } else if target == "~" {
            std::env::var("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::PathBuf::from("~"))
        } else {
            let p = std::path::Path::new(target);
            if p.is_absolute() {
                p.to_path_buf()
            } else if let Some(base) = cwd {
                base.join(p)
            } else if target.starts_with('-') {
                std::path::PathBuf::from(format!("./{target}"))
            } else {
                p.to_path_buf()
            }
        };

        #[cfg(target_os = "macos")]
        let res = {
            let mut cmd = std::process::Command::new(system_bin("open"));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let is_exec = path
                    .metadata()
                    .map(|m| m.is_file() && (m.permissions().mode() & 0o111 != 0))
                    .unwrap_or(false);
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if is_exec || ext.eq_ignore_ascii_case("app") || ext.eq_ignore_ascii_case("command") {
                    cmd.arg("-R");
                }
            }
            cmd.arg("--").arg(&path);
            spawn_detached(&mut cmd)
        };
        #[cfg(target_os = "linux")]
        let res = {
            let open_path = safe_path_for_linux_open(&path);
            spawn_detached(&mut linux_open_command(open_path))
        };

        if let Err(e) = res {
            eprintln!("litty: failed to open path {path:?}: {e}");
        }
    } else {
        #[cfg(target_os = "macos")]
        let res = spawn_detached(std::process::Command::new(system_bin("open")).arg("-u").arg(target));
        #[cfg(target_os = "linux")]
        let res = spawn_detached(&mut linux_open_command(target));

        if let Err(e) = res {
            eprintln!("litty: failed to open URL {target}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn linux_open_has_one_supported_target_argument() {
        for (target, expected) in [
            ("https://example.org/releases/v0.7.0", "https://example.org/releases/v0.7.0"),
            ("/tmp/a b.txt", "/tmp/a b.txt"), ("-a b.txt", "./-a b.txt"),
            ("./-file.txt", "./-file.txt"), ("relative/file.txt", "relative/file.txt"),
        ] {
            let command = super::linux_open_command(target);
            let args: Vec<_> = command.get_args().collect();
            assert_eq!(args, [std::ffi::OsStr::new(expected)]);
        }
    }

    /// Drives the production `open_target` through the real `xdg-open`; a stub
    /// `$BROWSER` records what the utility dispatches. Run by `scripts/linux/run.sh`.
    #[test]
    #[cfg(target_os = "linux")]
    #[ignore = "needs xdg-utils; run via scripts/linux/run.sh"]
    fn linux_xdg_open_dispatch_smoke() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("litty-xdg-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("dispatched.log");
        let stub = dir.join("stub-handler.sh");
        std::fs::write(&stub, format!("#!/bin/sh\nprintf '%s\\n' \"$1\" >> '{}'\n", log.display())).unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        let spaced = dir.join("a b.txt");
        let script = dir.join("run me.sh");
        std::fs::write(&spaced, "x").unwrap();
        std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.join("-dash.txt"), "x").unwrap();
        // SAFETY: ignored test, run alone, before any other thread reads the environment.
        unsafe {
            std::env::set_var("BROWSER", &stub);
            std::env::remove_var("DISPLAY");
            std::env::remove_var("WAYLAND_DISPLAY");
        }
        std::env::set_current_dir(&dir).unwrap();
        let url = "https://example.org/releases/v0.7.0";
        super::open_target(spaced.to_str().unwrap(), true, None);
        super::open_target(script.to_str().unwrap(), true, None);
        super::open_target("-dash.txt", true, None);
        super::open_target(url, false, None);
        let wanted = [
            spaced.to_str().unwrap().to_owned(),
            dir.to_str().unwrap().to_owned(),
            "./-dash.txt".to_owned(),
            url.to_owned(),
        ];
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut seen = Vec::new();
        while std::time::Instant::now() < deadline {
            seen = std::fs::read_to_string(&log).unwrap_or_default().lines().map(str::to_owned).collect();
            if seen.len() >= wanted.len() { break; }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        seen.sort();
        let mut wanted = wanted.to_vec();
        wanted.sort();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(seen, wanted, "executable/script paths must open their containing directory");
    }

    #[test]
    fn spawn_detached_rate_limit() {
        use super::{spawn_detached, IN_FLIGHT_DETACHED, MAX_CONCURRENT_DETACHED};
        use std::sync::atomic::Ordering;

        IN_FLIGHT_DETACHED.store(MAX_CONCURRENT_DETACHED, Ordering::SeqCst);
        let mut cmd = std::process::Command::new("/bin/echo");
        let res = spawn_detached(&mut cmd);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().kind(), std::io::ErrorKind::WouldBlock);

        IN_FLIGHT_DETACHED.store(0, Ordering::SeqCst);
    }

    #[test]
    #[cfg(unix)]
    fn safe_path_for_linux_open_guards_executables() {
        use super::safe_path_for_linux_open;
        use std::path::Path;

        // Script extensions should resolve to parent directory
        let script = Path::new("/home/user/project/setup.sh");
        assert_eq!(safe_path_for_linux_open(script), Path::new("/home/user/project"));

        let desktop = Path::new("/usr/share/applications/editor.desktop");
        assert_eq!(safe_path_for_linux_open(desktop), Path::new("/usr/share/applications"));

        let bin_ext = Path::new("/tmp/installer.bin");
        assert_eq!(safe_path_for_linux_open(bin_ext), Path::new("/tmp"));

        // Regular non-executable documents should open directly
        let doc = Path::new("/home/user/notes.txt");
        assert_eq!(safe_path_for_linux_open(doc), doc);

        let img = Path::new("/home/user/picture.png");
        assert_eq!(safe_path_for_linux_open(img), img);

        // Real filesystem executable file (e.g. /bin/sh or /bin/ls)
        let sh_bin = Path::new("/bin/sh");
        if sh_bin.exists() {
            assert_eq!(safe_path_for_linux_open(sh_bin), Path::new("/bin"));
        }
    }
}
