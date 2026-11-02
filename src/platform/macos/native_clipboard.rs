//! NSPasteboard string access (the subprocess fallback is platform::clipboard).

use super::objc::*;

/// Read plain text directly from NSPasteboard without thread spawning or pbpaste subprocess.
pub fn clipboard_string() -> Option<String> {
    unsafe {
        let pb_cls = class(c"NSPasteboard")?;
        let pb = send_id(pb_cls, c"generalPasteboard")?;
        let str_type = ns_string("public.utf8-plain-text")?;
        let s = msg_send!(pb, c"stringForType:", str_type => Id; -> Id);
        if !s.is_null() {
            return ns_string_to_string(s);
        }
        let legacy_type = ns_string("NSStringPboardType")?;
        let s2 = msg_send!(pb, c"stringForType:", legacy_type => Id; -> Id);
        if !s2.is_null() {
            return ns_string_to_string(s2);
        }
        None
    }
}

/// Write plain text directly to NSPasteboard without thread spawning or pbcopy subprocess.
pub fn clipboard_write_string(s: &str) -> bool {
    unsafe {
        let Some(pb_cls) = class(c"NSPasteboard") else { return false };
        let Some(pb) = send_id(pb_cls, c"generalPasteboard") else { return false };
        let _ = msg_send!(pb, c"clearContents"; -> isize);
        let Some(str_val) = ns_string(s) else { return false };
        let Some(str_type) = ns_string("public.utf8-plain-text") else { return false };
        msg_send!(pb, c"setString:forType:", str_val => Id, str_type => Id; -> bool)
    }
}
