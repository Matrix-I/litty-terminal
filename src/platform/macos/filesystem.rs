//! Atomic activation of a staged application bundle on the same volume.

use std::{ffi::CString, io, os::unix::ffi::OsStrExt, path::Path};

pub fn exchange_directories(staged: &Path, installed: &Path) -> io::Result<()> {
    // Do not swap a symlink or a regular file into the application's location.
    for path in [staged, installed] {
        if !std::fs::symlink_metadata(path)?.is_dir() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "bundle must be a directory"));
        }
    }
    let staged = CString::new(staged.as_os_str().as_bytes())?;
    let installed = CString::new(installed.as_os_str().as_bytes())?;
    // SAFETY: both paths remain NUL-terminated and valid during the call.
    let result = unsafe {
        libc::renameatx_np(libc::AT_FDCWD, staged.as_ptr(), libc::AT_FDCWD, installed.as_ptr(), libc::RENAME_SWAP)
    };
    if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_exchange_replaces_entire_tree_and_preserves_old_tree() {
        let dir = std::env::temp_dir().join(format!("litty-bundle-exchange-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let installed = dir.join("installed.app");
        let staged = dir.join("staged.app");
        std::fs::create_dir(&installed).unwrap();
        std::fs::create_dir(&staged).unwrap();
        std::fs::write(installed.join("obsolete"), "old").unwrap();
        std::fs::write(staged.join("current"), "new").unwrap();
        exchange_directories(&staged, &installed).unwrap();
        assert_eq!(std::fs::read(installed.join("current")).unwrap(), b"new");
        assert!(!installed.join("obsolete").exists());
        assert_eq!(std::fs::read(staged.join("obsolete")).unwrap(), b"old");
        assert!(exchange_directories(&dir.join("missing"), &installed).is_err());
        assert_eq!(std::fs::read(installed.join("current")).unwrap(), b"new");
        let link = dir.join("link");
        std::os::unix::fs::symlink(&staged, &link).unwrap();
        assert!(exchange_directories(&link, &installed).is_err());
        assert_eq!(std::fs::read(installed.join("current")).unwrap(), b"new");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
