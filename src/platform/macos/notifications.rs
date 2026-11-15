//! Native notifications; callbacks only enqueue exact-origin activation work.
use std::{ffi::{c_char, CStr, CString}, io, sync::Arc};

type Handler = Arc<dyn Fn(String, u64) + Send + Sync>;
static HANDLER: parking_lot::Mutex<Option<Handler>> = parking_lot::Mutex::new(None);
unsafe extern "C" {
    fn litty_notifications_init(callback: extern "C" fn(*const c_char, u64)) -> bool;
    fn litty_notification_startup() -> bool;
    fn litty_notifications_shutdown();
    #[cfg(test)]
    fn litty_notification_test_callbacks() -> bool;
    #[cfg(test)]
    fn litty_notification_test_late_authorization() -> bool;
    #[cfg(test)]
    fn litty_notification_test_selective_shutdown() -> bool;
    fn litty_notification_consume_reopen() -> bool;
    fn litty_notification_show(origin: *const c_char, pane: u64, title: *const c_char, body: *const c_char) -> i32;
}
fn valid_origin(origin: &str) -> bool {
    let pieces: Vec<_> = origin.split('-').collect();
    pieces.len() == 3 && pieces[0].len() == 16 && !pieces[1].is_empty() && pieces[1].len() <= 8 && pieces[2].len() == 32
        && pieces.iter().all(|piece| piece.bytes().all(|b| b.is_ascii_hexdigit()))
}
extern "C" fn response(origin: *const c_char, pane: u64) {
    // The native bridge supplies a validated, bounded, NUL-terminated UTF-8 string
    // whose lifetime covers this synchronous call. No Rust panic crosses the ABI.
    let result = std::panic::catch_unwind(|| {
        if origin.is_null() || pane == 0 { return; }
        let Ok(origin) = (unsafe { CStr::from_ptr(origin) }).to_str() else { return; };
        if !valid_origin(origin) { return; }
        let handler = HANDLER.lock().clone();
        if let Some(handler) = handler { handler(origin.to_owned(), pane); }
    });
    if result.is_err() { eprintln!("litty: notification response callback failed"); }
}
pub fn set_notification_handler(handler: Option<Handler>) { *HANDLER.lock() = handler; }
pub fn init_notifications() -> io::Result<()> {
    if unsafe { litty_notifications_init(response) } { Ok(()) }
    else { Err(io::Error::new(io::ErrorKind::Unsupported, "native notifications require an application bundle")) }
}
pub fn shutdown_notifications() { unsafe { litty_notifications_shutdown() } }
pub fn startup_is_notification() -> bool { unsafe { litty_notification_startup() } }
pub fn consume_notification_reopen() -> bool { unsafe { litty_notification_consume_reopen() } }
pub fn show_pane_notification(origin: &str, pane: u64, title: &str, body: &str) -> io::Result<()> {
    if !valid_origin(origin) || pane == 0 || title.len() > 4096 || body.len() > 16384 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid notification payload"));
    }
    let string = |text| CString::new(text).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "notification contains NUL"));
    let (origin, title, body) = (string(origin)?, string(title)?, string(body)?);
    match unsafe { litty_notification_show(origin.as_ptr(), pane, title.as_ptr(), body.as_ptr()) } {
        0 => Ok(()),
        1 => Err(io::Error::new(io::ErrorKind::Unsupported, "native notifications require an application bundle")),
        _ => Err(io::Error::new(io::ErrorKind::WouldBlock, "notification delivery queue is full")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_notification_survives_a_late_human_authorization() {
        assert!(unsafe { litty_notification_test_late_authorization() });
    }
    #[test]
    fn shutdown_withdraws_only_owned_notifications() {
        assert!(unsafe { litty_notification_test_selective_shutdown() });
    }
    #[test]
    fn native_callbacks_route_once_complete_all_paths_and_consume_reopen() {
        assert!(unsafe { litty_notification_test_callbacks() });
    }
    #[test]
    fn notification_boundaries_reject_bad_routes_and_text_before_native_submission() {
        let valid = "0000000000000001-1-00000000000000000000000000000001";
        assert!(valid_origin(valid));
        for origin in ["../bad", "", "0-1-aa", "0000000000000001-1-🙂"] { assert!(!valid_origin(origin)); }
        assert_eq!(show_pane_notification(valid, 0, "title", "body").unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(show_pane_notification(valid, 1, "\0", "body").unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(show_pane_notification(valid, 1, "title", &"x".repeat(16385)).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
}
