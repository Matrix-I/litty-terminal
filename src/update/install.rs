//! Choosing the platform build, caching verified downloads, and replacing the running binary.

use std::fs;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;

use super::client::download_file;
#[cfg(target_os = "macos")]
use super::resolve_system_bin;
use super::updates_dir;
use super::verify::{expected_hash, signature_asset, verify_checksum, verify_signature};
use super::{ReleaseAsset, ReleaseInfo, SUMS_NAME};

/// Detect if current executable is running inside an `.app` bundle on macOS.
#[cfg(target_os = "macos")]
pub(super) fn find_macos_app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut current = exe.parent()?;
    while let Some(parent) = current.parent() {
        if current.extension().is_some_and(|ext| ext == "app") {
            return Some(current.to_path_buf());
        }
        current = parent;
    }
    None
}

/// Release asset names for `os`/`arch` (`std::env::consts` naming), most specific first.
/// Releases up to v0.3.0 published unsuffixed names, built for one architecture only
/// (arm64 on macOS, x86_64 on Linux); they are offered only to that architecture.
pub(super) fn asset_candidates(os: &str, arch: &str, app_bundle: bool) -> Vec<String> {
    let (stem, ext, legacy_arch) = match (os, app_bundle) {
        ("macos", true) => ("Litty-macos", ".zip", "aarch64"),
        ("macos", false) => ("litty-macos", "", "aarch64"),
        _ => ("litty-linux", "", "x86_64"),
    };
    let mut names = vec![format!("{stem}-{arch}{ext}")];
    if arch == legacy_arch {
        names.push(format!("{stem}{ext}"));
    }
    names
}

/// Asset names appropriate for the current platform and installation.
pub(super) fn platform_asset_candidates() -> Vec<String> {
    #[cfg(target_os = "macos")]
    let app_bundle = find_macos_app_bundle().is_some();
    #[cfg(not(target_os = "macos"))]
    let app_bundle = false;
    asset_candidates(std::env::consts::OS, std::env::consts::ARCH, app_bundle)
}

/// First candidate the release both ships and lists in its checksum file.
pub(super) fn pick_asset<'a>(release: &'a ReleaseInfo, sums: &str, candidates: &[String]) -> Result<&'a ReleaseAsset, String> {
    candidates
        .iter()
        .find_map(|name| release.assets.iter().find(|a| &a.name == name && expected_hash(sums, name).is_some()))
        .ok_or_else(|| {
            format!(
                "release {} has no build for {}/{} (looked for {})",
                release.tag_name,
                std::env::consts::OS,
                std::env::consts::ARCH,
                candidates.join(", ")
            )
        })
}

pub(super) fn macho_arch(cpu_type: u32) -> Option<&'static str> {
    match cpu_type {
        0x0100_0007 => Some("x86_64"),
        0x0100_000c => Some("aarch64"),
        _ => None,
    }
}

pub(super) fn elf_arch(machine: u16) -> Option<&'static str> {
    match machine {
        62 => Some("x86_64"),
        183 => Some("aarch64"),
        _ => None,
    }
}

/// Architectures (`std::env::consts::ARCH` naming) an ELF, Mach-O or universal binary
/// header is built for. Empty when the header is not a recognised executable.
pub(super) fn binary_arches(h: &[u8]) -> Vec<&'static str> {
    let u16_at = |o: usize, be: bool| {
        h.get(o..o + 2).map(|b| if be { u16::from_be_bytes([b[0], b[1]]) } else { u16::from_le_bytes([b[0], b[1]]) })
    };
    let u32_at = |o: usize, be: bool| {
        h.get(o..o + 4).map(|b| {
            let b = [b[0], b[1], b[2], b[3]];
            if be { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }
        })
    };
    match h.get(..4) {
        // ELF: e_machine at offset 18, in the byte order given by EI_DATA (2 = big endian).
        Some([0x7f, b'E', b'L', b'F']) => u16_at(18, h.get(5) == Some(&2)).and_then(elf_arch).into_iter().collect(),
        // 64-bit little-endian Mach-O: cputype follows the magic.
        Some([0xcf, 0xfa, 0xed, 0xfe]) => u32_at(4, false).and_then(macho_arch).into_iter().collect(),
        // Universal binary (big endian): nfat_arch, then fat_arch (20 B) or fat_arch_64 (32 B) entries.
        Some([0xca, 0xfe, 0xba, kind @ (0xbe | 0xbf)]) => {
            let stride = if *kind == 0xbe { 20 } else { 32 };
            let count = u32_at(4, true).unwrap_or(0).min(16) as usize;
            (0..count).filter_map(|i| u32_at(8 + i * stride, true)).filter_map(macho_arch).collect()
        }
        _ => Vec::new(),
    }
}

pub(super) fn ensure_binary_arch(header: &[u8], arch: &str) -> Result<(), String> {
    let found = binary_arches(header);
    if found.contains(&arch) {
        Ok(())
    } else if found.is_empty() {
        Err("downloaded update is not a recognised executable".to_string())
    } else {
        Err(format!("downloaded update is built for {}, but this machine is {arch}", found.join("/")))
    }
}

/// Refuse to install a binary that cannot run on this machine.
pub(super) fn check_binary_arch(path: &Path) -> Result<(), String> {
    let mut header = Vec::with_capacity(4096);
    fs::File::open(path)
        .and_then(|f| f.take(4096).read_to_end(&mut header))
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    ensure_binary_arch(&header, std::env::consts::ARCH)
}

/// Exclusive lock over the updates directory, shared by the app's threads and by
/// `litty +update` in another process (`flock` locks conflict across open files).
/// Released when dropped.
pub(super) struct UpdateLock(#[allow(dead_code)] fs::File);

impl UpdateLock {
    pub(super) fn acquire(dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(".lock"))
            .map_err(|e| format!("cannot open update lock: {e}"))?;
        // SAFETY: flock on a file descriptor we own for the lifetime of `file`.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("another Litty update is already in progress".to_string());
        }
        Ok(Self(file))
    }
}

/// Download (or reuse) the release asset for `candidates` into `dir/<tag>/` and verify it.
/// Returns the verified file and its asset name. The caller holds the `UpdateLock`.
pub(crate) fn download_and_verify_in(
    dir: &Path,
    release: &ReleaseInfo,
    candidates: &[String],
) -> Result<(PathBuf, String), String> {
    let release_dir = dir.join(&release.tag_name);
    let sums_path = release_dir.join(SUMS_NAME);

    // Reuse a build that an earlier `auto-update = download` run already fetched,
    // but only together with a signature that still verifies.
    if let Ok(sums) = fs::read_to_string(&sums_path)
        && let Ok(asset) = pick_asset(release, &sums, candidates) {
            let path = release_dir.join(&asset.name);
            if path.is_file() && verify_checksum(&sums, &asset.name, &path).is_ok()
                && let Some(sig_asset) = signature_asset(release) {
                    let sig_path = release_dir.join(&sig_asset.name);
                    if sig_path.is_file() {
                        verify_signature(&sums_path, &sig_path)?;
                        return Ok((path, asset.name.clone()));
                    }
                }
        }

    let sums_asset = release
        .assets
        .iter()
        .find(|a| a.name == SUMS_NAME)
        .ok_or_else(|| format!("release {} does not contain {SUMS_NAME}", release.tag_name))?;
    // Unsigned releases are refused before anything is downloaded (fail closed).
    let sig_asset = signature_asset(release).ok_or_else(|| {
        format!("release {} is not signed (no {SUMS_NAME}.minisig / .sig / .asc); refusing to install it", release.tag_name)
    })?;
    fs::create_dir_all(&release_dir).map_err(|e| format!("cannot create {}: {e}", release_dir.display()))?;
    download_file(&sums_asset.download_url, &sums_path)?;
    let sums = fs::read_to_string(&sums_path).map_err(|e| format!("cannot read {SUMS_NAME}: {e}"))?;

    let sig_path = release_dir.join(&sig_asset.name);
    download_file(&sig_asset.download_url, &sig_path)?;
    if let Err(e) = verify_signature(&sums_path, &sig_path) {
        let _ = fs::remove_file(&sums_path);
        let _ = fs::remove_file(&sig_path);
        return Err(e);
    }

    let asset = pick_asset(release, &sums, candidates)?;
    let path = release_dir.join(&asset.name);
    // Only a verified file ever carries the final name, so the reuse check above can trust it.
    let partial = release_dir.join(format!("{}.part", asset.name));
    download_file(&asset.download_url, &partial)?;
    if let Err(e) = verify_checksum(&sums, &asset.name, &partial) {
        let _ = fs::remove_file(&partial);
        return Err(e);
    }
    fs::rename(&partial, &path).map_err(|e| format!("cannot store verified download: {e}"))?;
    Ok((path, asset.name.clone()))
}

/// Remove everything in the updates directory except the lock and the `keep` release.
pub(super) fn prune_other_releases(dir: &Path, keep: &str) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == keep || name == ".lock" {
            continue;
        }
        let path = entry.path();
        let _ = if path.is_dir() { fs::remove_dir_all(&path) } else { fs::remove_file(&path) };
    }
}

/// Download, verify and install `release`, then drop its cached download.
pub(super) fn install_release(release: &ReleaseInfo) -> Result<(), String> {
    let dir = updates_dir();
    let _lock = UpdateLock::acquire(&dir)?;
    let (file, name) = download_and_verify_in(&dir, release, &platform_asset_candidates())?;
    apply_update(&file, &name)?;
    let _ = fs::remove_dir_all(dir.join(&release.tag_name));
    Ok(())
}

/// Download and verify `release` without installing it (`auto-update = download`).
pub(super) fn prefetch_release(release: &ReleaseInfo) -> Result<(), String> {
    let dir = updates_dir();
    let _lock = UpdateLock::acquire(&dir)?;
    prune_other_releases(&dir, &release.tag_name);
    download_and_verify_in(&dir, release, &platform_asset_candidates()).map(drop)
}

/// Apply verified update to the current binary or macOS app bundle.
pub fn apply_update(downloaded_file: &Path, asset_name: &str) -> Result<(), String> {
    let current_exe = std::env::current_exe().map_err(|e| format!("cannot determine current executable: {e}"))?;
    let exe_str = current_exe.to_string_lossy();

    // Prevent overwriting local development/debug binaries
    if exe_str.contains("/target/debug/") || exe_str.contains("/target/release/") {
        return Err(format!(
            "Litty is running from a build target ({exe_str}).\nSelf-update is disabled for dev builds. Run `cargo build --release` instead."
        ));
    }

    #[cfg(target_os = "macos")]
    if asset_name.ends_with(".zip") {
        let app_path = find_macos_app_bundle().ok_or("Litty is not running from an .app bundle")?;
        let staging_dir = downloaded_file.with_file_name("staging");
        let _ = fs::remove_dir_all(&staging_dir);
        let _ = fs::create_dir_all(&staging_dir);

        struct StagingCleanup<'a>(&'a Path);
        impl Drop for StagingCleanup<'_> {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(self.0);
            }
        }
        let _guard = StagingCleanup(&staging_dir);

        // Unzip using system ditto for macOS resource fork and symlink fidelity
        let status = Command::new("/usr/bin/ditto")
            .args(["-xk"])
            .arg(downloaded_file)
            .arg(&staging_dir)
            .status()
            .map_err(|e| format!("ditto unpack failed: {e}"))?;

        if !status.success() {
            return Err(format!("failed to extract {asset_name}"));
        }

        let staged_app = staging_dir.join("Litty.app");
        if !staged_app.exists() {
            return Err("extracted archive did not contain Litty.app".to_string());
        }
        let bin_path = staged_app.join("Contents").join("MacOS").join("litty");
        check_binary_arch(&bin_path)?;

        // Sync updated contents to app_path
        let status = Command::new("/usr/bin/ditto")
            .arg(&staged_app)
            .arg(&app_path)
            .status()
            .map_err(|e| format!("failed to overwrite app bundle: {e}"))?;

        if !status.success() {
            return Err("failed to copy update into application directory (check permissions)".to_string());
        }

        // Ad-hoc sign so Gatekeeper does not block the updated bundle
        let sign_status = Command::new(resolve_system_bin("codesign"))
            .args(["--force", "--deep", "--sign", "-"])
            .arg(&app_path)
            .status()
            .map_err(|e| format!("failed to execute codesign on {app_path:?}: {e}"))?;
        if !sign_status.success() {
            return Err(format!("codesign failed on {app_path:?} with exit code {:?}", sign_status.code()));
        }

        let touch_status = Command::new(resolve_system_bin("touch"))
            .arg(&app_path)
            .status()
            .map_err(|e| format!("failed to execute touch on {app_path:?}: {e}"))?;
        if !touch_status.success() {
            return Err(format!("touch failed on {app_path:?} with exit code {:?}", touch_status.code()));
        }
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    let _ = asset_name;

    // Standalone binary replacement
    check_binary_arch(downloaded_file)?;
    let temp_dest = current_exe.with_extension("new");
    stage_binary(downloaded_file, &temp_dest).map_err(|e| format!("cannot copy binary: {e}"))?;

    struct FileCleanup<'a>(&'a Path, bool);
    impl Drop for FileCleanup<'_> {
        fn drop(&mut self) {
            if self.1 {
                let _ = fs::remove_file(self.0);
            }
        }
    }
    let mut file_guard = FileCleanup(&temp_dest, true);

    #[cfg(target_os = "macos")]
    {
        let sign_status = Command::new(resolve_system_bin("codesign"))
            .args(["--force", "--deep", "--sign", "-"])
            .arg(&temp_dest)
            .status()
            .map_err(|e| format!("failed to execute codesign on {temp_dest:?}: {e}"))?;
        if !sign_status.success() {
            return Err(format!("codesign failed on {temp_dest:?} with exit code {:?}", sign_status.code()));
        }
    }

    fs::rename(&temp_dest, &current_exe).map_err(|e| format!("failed to replace binary {exe_str}: {e}"))?;
    file_guard.1 = false;
    Ok(())
}

/// Copies `src` to a fresh executable file at `dest`. A stale or planted entry
/// at `dest` is unlinked (never followed) and the new file is created with
/// `O_EXCL | O_NOFOLLOW`, so the copy cannot write through a symlink (CWE-59).
pub(crate) fn stage_binary(src: &Path, dest: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    match fs::remove_file(dest) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut input = fs::File::open(src)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o755)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dest)?;
    std::io::copy(&mut input, &mut output)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::verify::sha256_digest;
    use crate::update::{release, temp_dir};

    #[test]
    fn test_asset_candidates_per_arch() {
        assert_eq!(asset_candidates("macos", "aarch64", true), ["Litty-macos-aarch64.zip", "Litty-macos.zip"]);
        assert_eq!(asset_candidates("macos", "x86_64", true), ["Litty-macos-x86_64.zip"]);
        assert_eq!(asset_candidates("macos", "aarch64", false), ["litty-macos-aarch64", "litty-macos"]);
        assert_eq!(asset_candidates("macos", "x86_64", false), ["litty-macos-x86_64"]);
        assert_eq!(asset_candidates("linux", "x86_64", false), ["litty-linux-x86_64", "litty-linux"]);
        assert_eq!(asset_candidates("linux", "aarch64", false), ["litty-linux-aarch64"]);
    }

    #[test]
    fn test_pick_asset_requires_listing_in_sums() {
        let hash = "a".repeat(64);
        let r = release("v0.4.0", &["litty-linux-x86_64", "litty-linux", SUMS_NAME]);
        let candidates = asset_candidates("linux", "x86_64", false);
        // Only the legacy name is checksummed, so it wins despite being second.
        let sums = format!("{hash}  litty-linux\n");
        assert_eq!(pick_asset(&r, &sums, &candidates).unwrap().name, "litty-linux");
        let sums = format!("{hash}  litty-linux-x86_64\n{hash} *litty-linux\n");
        assert_eq!(pick_asset(&r, &sums, &candidates).unwrap().name, "litty-linux-x86_64");
        let err = pick_asset(&r, &sums, &asset_candidates("linux", "aarch64", false)).unwrap_err();
        assert!(err.contains("litty-linux-aarch64"), "{err}");
    }

    #[test]
    fn test_binary_arches() {
        let mut elf = vec![0u8; 64];
        elf[..6].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1]);
        elf[18..20].copy_from_slice(&62u16.to_le_bytes());
        assert_eq!(binary_arches(&elf), ["x86_64"]);
        elf[18..20].copy_from_slice(&183u16.to_le_bytes());
        assert_eq!(binary_arches(&elf), ["aarch64"]);

        let mut macho = vec![0xcf, 0xfa, 0xed, 0xfe];
        macho.extend_from_slice(&0x0100_000cu32.to_le_bytes());
        assert_eq!(binary_arches(&macho), ["aarch64"]);

        let mut fat = vec![0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 2];
        for cpu in [0x0100_0007u32, 0x0100_000c] {
            fat.extend_from_slice(&cpu.to_be_bytes());
            fat.extend_from_slice(&[0; 16]);
        }
        assert_eq!(binary_arches(&fat), ["x86_64", "aarch64"]);

        assert!(binary_arches(b"PK\x03\x04zip").is_empty());
        assert!(binary_arches(&[0x7f, b'E']).is_empty());
    }

    #[test]
    fn test_ensure_binary_arch() {
        let mut macho = vec![0xcf, 0xfa, 0xed, 0xfe];
        macho.extend_from_slice(&0x0100_000cu32.to_le_bytes());
        assert!(ensure_binary_arch(&macho, "aarch64").is_ok());
        let err = ensure_binary_arch(&macho, "x86_64").unwrap_err();
        assert!(err.contains("built for aarch64") && err.contains("x86_64"), "{err}");
        assert!(ensure_binary_arch(b"<html>", "x86_64").is_err());
    }

    #[test]
    fn test_update_lock_is_exclusive() {
        let dir = temp_dir("lock");
        let first = UpdateLock::acquire(&dir).unwrap();
        assert!(UpdateLock::acquire(&dir).is_err());
        drop(first);
        assert!(UpdateLock::acquire(&dir).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_download_reuses_verified_cache_without_network() {
        let dir = temp_dir("cache");
        let r = release("v9.9.9", &["litty-test", SUMS_NAME]);
        let release_dir = dir.join(&r.tag_name);
        fs::create_dir_all(&release_dir).unwrap();
        fs::write(release_dir.join("litty-test"), b"binary").unwrap();
        fs::write(release_dir.join(SUMS_NAME), format!("{}  litty-test\n", sha256_digest(b"binary"))).unwrap();

        // Unsigned release in cache must be rejected (fail closed).
        assert!(download_and_verify_in(&dir, &r, &["litty-test".to_string()]).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_prune_other_releases_keeps_lock_and_current() {
        let dir = temp_dir("prune");
        for sub in ["v0.3.0", "v0.4.0", "staging"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
        }
        fs::write(dir.join(".lock"), b"").unwrap();
        fs::write(dir.join("litty-macos"), b"old layout").unwrap();
        prune_other_releases(&dir, "v0.4.0");
        let mut left: Vec<_> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        left.sort();
        assert_eq!(left, [".lock", "v0.4.0"]);
        let _ = fs::remove_dir_all(&dir);
    }
}
