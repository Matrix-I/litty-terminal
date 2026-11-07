//! Background and interactive auto-update checks for Litty Terminal.
//!
//! Checks GitHub Releases for new versions, downloads release assets,
//! verifies SHA-256 checksums against `SHA256SUMS.txt`, and replaces
//! the running binary or `.app` bundle safely.

mod client;
mod install;
mod verify;

pub use client::fetch_latest_release;
pub use install::apply_update;
pub(crate) use install::download_and_verify_in;
use install::{install_release, platform_asset_candidates, prefetch_release, UpdateLock};

// Exercised by the integration-style checks in `security_poc`.
#[cfg(test)]
pub(crate) use client::parse_release_json;
#[cfg(test)]
pub(crate) use install::stage_binary;
#[cfg(test)]
pub use verify::sha256_digest;
#[cfg(test)]
pub(crate) use verify::verify_signature;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Update checking mode (Ghostty-compatible `auto-update` configuration).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum AutoUpdateMode {
    /// Periodically check in background and notify when an update is available.
    #[default]
    Check,
    /// Download and verify the update in background, then notify that it is ready.
    /// Installing still waits for the user (Check for Updates… or `litty +update`).
    Download,
    /// Do not check for updates automatically.
    Off,
}

/// Channel to track for updates.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum AutoUpdateChannel {
    #[default]
    Stable,
    Tip,
}

/// Parsed semantic version `major.minor.patch`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let trimmed = s.trim().trim_start_matches(['v', 'V']);
        // Handle prerelease tags like "0.3.1-beta.1"
        let core = trimmed.split_once('-').map_or(trimmed, |(c, _)| c);
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        Some(Self { major, minor, patch })
    }

    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Self { major: 0, minor: 0, patch: 0 })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Release asset metadata.
#[derive(Clone, Debug)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
}

/// GitHub release information.
#[derive(Clone, Debug)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub version: Version,
    pub html_url: String,
    pub assets: Vec<ReleaseAsset>,
}

pub(crate) const SUMS_NAME: &str = "SHA256SUMS.txt";

/// Path to cache directory `~/.cache/litty`.
fn cache_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".cache").join("litty")
    } else {
        std::env::temp_dir().join("litty")
    }
}

/// Directory holding downloaded releases, one subdirectory per tag.
fn updates_dir() -> PathBuf {
    cache_dir().join("updates")
}

/// Path to timestamp file recording the last update check.
fn last_check_file() -> PathBuf {
    cache_dir().join("last_update_check")
}

/// Check if at least 24 hours have elapsed since the last background update check.
pub fn is_check_due() -> bool {
    let path = last_check_file();
    let Ok(content) = fs::read_to_string(&path) else {
        return true;
    };
    let Ok(ts) = content.trim().parse::<u64>() else {
        return true;
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // 24 hours = 86400 seconds
    now.saturating_sub(ts) >= 86400
}

/// Record current timestamp as the last update check time.
pub fn mark_checked() {
    let dir = cache_dir();
    let _ = fs::create_dir_all(&dir);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let _ = fs::write(last_check_file(), now.to_string());
}

/// System binary path resolution with fallback.
fn resolve_system_bin(name: &'static str) -> &'static str {
    match name {
        "curl" => if Path::new("/usr/bin/curl").exists() { "/usr/bin/curl" } else { "curl" },
        "codesign" => if Path::new("/usr/bin/codesign").exists() { "/usr/bin/codesign" } else { "codesign" },
        "touch" => if Path::new("/usr/bin/touch").exists() { "/usr/bin/touch" } else { "touch" },
        _ => name,
    }
}

/// How to install an update on this platform, for notifications.
fn install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "Choose Litty ▸ Check for Updates… to install."
    } else {
        "Run `litty +update` to install."
    }
}

static CHECKING: AtomicBool = AtomicBool::new(false);
static INSTALLING: AtomicBool = AtomicBool::new(false);

/// Trigger an interactive check for updates (e.g. from macOS menu or keybinding).
/// Runs the network check on a background thread and posts the result back to the main thread.
pub fn start_interactive_check(
    proxy: crate::platform::gui::EventProxy<crate::AppEvent>,
    channel: AutoUpdateChannel,
) {
    if CHECKING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let res = fetch_latest_release(channel);
        CHECKING.store(false, Ordering::SeqCst);
        let _ = proxy.send_event(crate::AppEvent::UpdateCheckResult(res));
    });
}

/// Download, verify, and apply update on a background thread, posting the result back to the main thread.
/// Returns `false` (and does nothing) when an install is already running in this process.
pub fn start_install_update(
    release: ReleaseInfo,
    proxy: crate::platform::gui::EventProxy<crate::AppEvent>,
) -> bool {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return false;
    }
    std::thread::spawn(move || {
        let res = install_release(&release).map(|()| release.version);
        INSTALLING.store(false, Ordering::SeqCst);
        let _ = proxy.send_event(crate::AppEvent::UpdateInstallResult(res));
    });
    true
}

/// Startup check run in a background thread if `auto_update != Off`.
pub fn check_on_startup(mode: AutoUpdateMode, channel: AutoUpdateChannel) {
    if mode == AutoUpdateMode::Off || !is_check_due() {
        return;
    }

    std::thread::spawn(move || {
        let current = Version::current();
        let Ok(release) = fetch_latest_release(channel) else {
            return;
        };
        mark_checked();

        if release.version <= current {
            return;
        }

        match mode {
            AutoUpdateMode::Check => {
                crate::platform::process::show_notification(
                    "Litty Update Available",
                    &format!("v{} is available. {}", release.version, install_hint()),
                );
            }
            AutoUpdateMode::Download => match prefetch_release(&release) {
                Ok(()) => crate::platform::process::show_notification(
                    "Litty Update Ready",
                    &format!("v{} has been downloaded and verified. {}", release.version, install_hint()),
                ),
                Err(e) => eprintln!("litty: background update download failed: {e}"),
            },
            AutoUpdateMode::Off => {}
        }
    });
}

/// CLI command `litty +check-update`.
pub fn cli_check_update() {
    let current = Version::current();
    println!("Litty Terminal — Update Check");
    println!("Current version: v{current}");
    print!("Checking GitHub releases... ");
    match fetch_latest_release(AutoUpdateChannel::Stable) {
        Ok(release) => {
            println!("done.");
            println!("Latest version:  v{}", release.version);
            if release.version > current {
                println!("\n⚡ An update is available: v{current} -> v{}", release.version);
                println!("Release URL: {}", release.html_url);
                println!("\nRun `litty +update` to automatically download and install this update.");
            } else {
                println!("\n✓ You are running the latest version of Litty.");
            }
        }
        Err(e) => {
            println!("failed.");
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}

/// CLI command `litty +update` (supports `--force`).
pub fn cli_update(force: bool) {
    let current = Version::current();
    println!("Litty Terminal — Self-Updater");
    println!("Current version: v{current}");
    print!("Checking for new releases... ");
    let release = match fetch_latest_release(AutoUpdateChannel::Stable) {
        Ok(r) => {
            println!("done (found v{}).", r.version);
            r
        }
        Err(e) => {
            println!("failed.");
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    if release.version <= current && !force {
        println!("✓ Litty is already up to date (v{current}).");
        println!("(To reinstall anyway, run `litty +update --force`)");
        return;
    }

    let dir = updates_dir();
    let _lock = UpdateLock::acquire(&dir).unwrap_or_else(|e| {
        eprintln!("Error: {e}");
        std::process::exit(1);
    });

    println!("Downloading the {}/{} build...", std::env::consts::OS, std::env::consts::ARCH);
    let (dest_file, name) = match download_and_verify_in(&dir, &release, &platform_asset_candidates()) {
        Ok(res) => {
            println!("✓ `{}` ready. SHA-256 checksum verified.", res.1);
            res
        }
        Err(e) => {
            eprintln!("Error downloading/verifying update: {e}");
            std::process::exit(1);
        }
    };

    print!("Installing update... ");
    match apply_update(&dest_file, &name) {
        Ok(()) => {
            let _ = fs::remove_dir_all(dir.join(&release.tag_name));
            println!("done!");
            println!("\n🎉 Successfully updated Litty to v{}!", release.version);
            println!("Please restart Litty to use the new version.");
        }
        Err(e) => {
            println!("failed.");
            eprintln!("Error applying update: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
fn release(tag: &str, assets: &[&str]) -> ReleaseInfo {
    ReleaseInfo {
        tag_name: tag.to_string(),
        version: Version::parse(tag).unwrap(),
        html_url: format!("{}{tag}", client::WEB_TAG_PREFIX),
        assets: assets
            .iter()
            .map(|n| ReleaseAsset { name: n.to_string(), download_url: format!("https://invalid.invalid/{n}") })
            .collect(),
    }
}

#[cfg(test)]
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("litty-update-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parse() {
        assert_eq!(Version::parse("v0.3.0"), Some(Version { major: 0, minor: 3, patch: 0 }));
        assert_eq!(Version::parse("0.3.1"), Some(Version { major: 0, minor: 3, patch: 1 }));
        assert_eq!(Version::parse("v1.0.0-rc1"), Some(Version { major: 1, minor: 0, patch: 0 }));
        assert!(Version::parse("invalid").is_none());
    }

    #[test]
    fn test_version_comparison() {
        let v030 = Version { major: 0, minor: 3, patch: 0 };
        let v031 = Version { major: 0, minor: 3, patch: 1 };
        let v100 = Version { major: 1, minor: 0, patch: 0 };
        assert!(v031 > v030);
        assert!(v100 > v031);
        assert_eq!(v030, Version { major: 0, minor: 3, patch: 0 });
    }
}
