//! macOS bundle activation has no Linux counterpart; standalone updates use rename.

#[allow(dead_code)]
pub fn exchange_directories(_staged: &std::path::Path, _installed: &std::path::Path) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "application bundle exchange is macOS-only"))
}
