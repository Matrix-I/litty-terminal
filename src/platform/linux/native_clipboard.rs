//! NSPasteboard has no Linux twin; the subprocess fallback is platform::clipboard.

pub fn clipboard_string() -> Option<String> {
    None
}

pub fn clipboard_write_string(_s: &str) -> bool {
    false
}
