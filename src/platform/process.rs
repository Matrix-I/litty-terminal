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

/// Extensions a click opens with their default app: viewers and editors that do
/// not run the file. Everything else (scripts, bundles, `.terminal`, `.webloc`,
/// installers, web pages) is only revealed, since a program in the terminal
/// chooses both a link's target and the text shown for it.
const OPEN_EXTENSIONS: &[&str] = &[
    // Text, data and source code.
    "txt", "text", "md", "markdown", "rst", "adoc", "org", "log", "csv", "tsv", "json", "jsonl",
    "yaml", "yml", "toml", "ini", "cfg", "conf", "xml", "diff", "patch", "lock", "rs", "c", "h",
    "cc", "cpp", "cxx", "hpp", "hh", "go", "java", "kt", "swift", "cs", "scala", "hs", "zig",
    "dart", "ts", "tsx", "css", "scss", "sql", "proto", "graphql", "wgsl", "glsl", "metal",
    // Documents and media.
    "pdf", "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif", "avif",
    "ico", "mp3", "wav", "flac", "ogg", "opus", "m4a", "aac", "aif", "aiff", "mp4", "m4v", "mov",
    "mkv", "webm", "avi",
];

/// What a click does with a local path.
#[derive(Debug, PartialEq, Eq)]
enum PathAction {
    /// Hand this resolved path to its default app. Opening the path that was
    /// checked, not the clicked name, means a link swapped after the check
    /// cannot redirect the open.
    Open(std::path::PathBuf),
    /// Show it in its folder without opening it.
    Reveal,
}

fn path_action(path: &std::path::Path) -> PathAction {
    use std::os::unix::fs::PermissionsExt;
    let ext = |p: &std::path::Path| p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let allowed = |e: Option<String>| e.is_some_and(|e| OPEN_EXTENSIONS.contains(&e.as_str()));
    // The system acts on a symlink's target, so judge the target as well as the name.
    let Ok(real) = std::fs::canonicalize(path) else { return PathAction::Reveal };
    let Ok(meta) = real.metadata() else { return PathAction::Reveal };
    let open = if meta.is_dir() {
        // On macOS a directory with an extension is a bundle (`.app`, `.workflow`) that `open` runs.
        !cfg!(target_os = "macos") || (ext(path).is_none() && ext(&real).is_none())
    } else {
        meta.is_file() && meta.permissions().mode() & 0o111 == 0 && allowed(ext(path)) && allowed(ext(&real))
    };
    if open { PathAction::Open(real) } else { PathAction::Reveal }
}

/// The command that opens or reveals a clicked local path, never through a shell.
fn path_command(path: &std::path::Path) -> std::process::Command {
    let action = path_action(path);
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new(system_bin("open"));
        match &action {
            PathAction::Open(real) => cmd.arg("--").arg(real),
            PathAction::Reveal => cmd.arg("-R").arg("--").arg(path),
        };
        cmd
    }
    #[cfg(target_os = "linux")]
    {
        // xdg-open has no reveal, so open the containing folder instead.
        let target = match (&action, path.parent()) {
            (PathAction::Open(real), _) => real.as_path(),
            (PathAction::Reveal, Some(dir)) if !dir.as_os_str().is_empty() => dir,
            (PathAction::Reveal, _) => std::path::Path::new("."),
        };
        linux_open_command(target)
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

        let res = spawn_detached(&mut path_command(&path));

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
            // Opened by its resolved absolute path, so no `./` guard is needed.
            dir.join("-dash.txt").to_str().unwrap().to_owned(),
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

    /// Files and links in a fresh temp directory, removed on drop.
    struct Fixture(std::path::PathBuf);

    impl Fixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("litty-open-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, name: &str, mode: u32) -> std::path::PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let path = self.0.join(name);
            std::fs::write(&path, "x").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn clicked_paths_open_only_allowlisted_documents() {
        use super::{PathAction::{Open, Reveal}, path_action};
        let fx = Fixture::new("allowlist");
        for name in ["notes.txt", "README.md", "main.rs", "photo.PNG", "clip.mov", "paper.pdf"] {
            let path = fx.file(name, 0o644);
            assert_eq!(path_action(&path), Open(std::fs::canonicalize(&path).unwrap()), "{name}");
        }
        // Types whose default handler runs code or opens a URL or installer.
        for name in [
            "x.terminal", "x.webloc", "x.inetloc", "x.fileloc", "x.jar", "x.pkg", "x.command",
            "x.sh", "x.py", "x.desktop", "x.bin", "x.html", "x.svg", "Makefile", "X.TERMINAL",
        ] {
            assert_eq!(path_action(&fx.file(name, 0o644)), Reveal, "{name}");
        }
        assert_eq!(path_action(&fx.file("run.txt", 0o755)), Reveal, "executable bit");
        assert_eq!(path_action(&fx.0.join("missing.txt")), Reveal);

        // A symlink is judged by both its own name and its target.
        let evil = fx.file("payload.terminal", 0o644);
        let disguised = fx.0.join("innocent.txt");
        std::os::unix::fs::symlink(&evil, &disguised).unwrap();
        assert_eq!(path_action(&disguised), Reveal);
        let renamed = fx.0.join("alias.terminal");
        std::os::unix::fs::symlink(fx.0.join("notes.txt"), &renamed).unwrap();
        assert_eq!(path_action(&renamed), Reveal);
        // The resolved target is what gets opened, not the clicked name.
        let doc = fx.0.join("doc-link.md");
        std::os::unix::fs::symlink(fx.0.join("README.md"), &doc).unwrap();
        assert_eq!(path_action(&doc), Open(std::fs::canonicalize(fx.0.join("README.md")).unwrap()));

        let src = fx.0.join("src");
        std::fs::create_dir(&src).unwrap();
        assert_eq!(path_action(&src), Open(std::fs::canonicalize(&src).unwrap()));
        // On macOS `open` launches a directory with an extension as a bundle,
        // also when it is reached through a symlink with a plain name.
        let app = fx.0.join("Tool.app");
        std::fs::create_dir(&app).unwrap();
        let tool = fx.0.join("tool");
        std::os::unix::fs::symlink(&app, &tool).unwrap();
        for path in [&app, &tool] {
            let bundle = if cfg!(target_os = "macos") { Reveal } else { Open(std::fs::canonicalize(&app).unwrap()) };
            assert_eq!(path_action(path), bundle, "{path:?}");
        }
    }

    #[test]
    fn clicked_path_commands_reveal_instead_of_open() {
        let fx = Fixture::new("commands");
        let doc = fx.file("notes.txt", 0o644);
        let evil = fx.file("x.webloc", 0o644);
        let args = |path: &std::path::Path| -> Vec<std::ffi::OsString> {
            super::path_command(path).get_args().map(Into::into).collect()
        };
        #[cfg(target_os = "macos")]
        {
            let real = std::fs::canonicalize(&doc).unwrap();
            assert_eq!(args(&doc), [std::ffi::OsString::from("--"), real.into()]);
            assert_eq!(args(&evil), [std::ffi::OsString::from("-R"), "--".into(), evil.clone().into()]);
        }
        #[cfg(target_os = "linux")]
        {
            assert_eq!(args(&doc), [std::fs::canonicalize(&doc).unwrap().into_os_string()]);
            assert_eq!(args(&evil), [fx.0.clone().into_os_string()]);
            // A bare relative name reveals the current directory, not an empty argument.
            assert_eq!(args(std::path::Path::new("x.webloc")), [std::ffi::OsString::from(".")]);
        }
    }
}
