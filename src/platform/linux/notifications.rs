//! Linux desktop notifications go over the session bus (`notify_bus`), so a
//! click reaches the notification router and selects its pane;
//! `notify-send` remains the fallback without a bus or notification server.
use std::{io, sync::Arc};

type Handler = Arc<dyn Fn(String, u64) + Send + Sync>;
static HANDLER: parking_lot::Mutex<Option<Handler>> = parking_lot::Mutex::new(None);

pub fn set_notification_handler(handler: Option<Handler>) { *HANDLER.lock() = handler; }
/// Hand a click to the notification router; dropped when no router runs.
pub(crate) fn deliver_notification_click(origin: String, pane: u64) {
    let handler = HANDLER.lock().clone();
    if let Some(handler) = handler { handler(origin, pane); }
}
#[allow(dead_code)] // Native launch initialization is macOS-only.
pub fn init_notifications() -> io::Result<()> { Ok(()) }
#[allow(dead_code)] // Native notification cleanup is macOS-only.
pub fn shutdown_notifications() {}
/// A click on a Linux notification never relaunches the app.
pub fn startup_is_notification() -> bool { false }
#[allow(dead_code)] // Linux has no AppKit reopen callback.
pub fn consume_notification_reopen() -> bool { false }
fn valid_origin(origin: &str) -> bool {
    let pieces: Vec<_> = origin.split('-').collect();
    pieces.len() == 3 && pieces[0].len() == 16 && !pieces[1].is_empty() && pieces[1].len() <= 8 && pieces[2].len() == 32
        && pieces.iter().all(|piece| piece.bytes().all(|b| b.is_ascii_hexdigit()))
}
/// `Unsupported` without a notification bus: the caller falls back to `notify-send`.
pub fn show_pane_notification(origin: &str, pane: u64, title: &str, body: &str) -> io::Result<()> {
    if !valid_origin(origin) || pane == 0 || title.len() > 4096 || body.len() > 16384
        || title.contains('\0') || body.contains('\0')
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid notification payload"));
    }
    super::native::notify_bus::show(origin, pane, title, body)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_boundaries_reject_bad_routes_and_text_before_the_bus() {
        let valid = "0000000000000001-1-00000000000000000000000000000001";
        assert!(valid_origin(valid));
        for origin in ["../bad", "", "0-1-aa", "0000000000000001-1-🙂"] {
            assert!(!valid_origin(origin));
        }
        let rejects = |origin: &str, pane: u64, title: &str, body: &str| {
            show_pane_notification(origin, pane, title, body).unwrap_err().kind() == io::ErrorKind::InvalidInput
        };
        assert!(rejects("../bad", 1, "title", "body"));
        assert!(rejects(valid, 0, "title", "body"));
        assert!(rejects(valid, 1, "\0", "body"));
        assert!(rejects(valid, 1, "title", "a\0b"));
        assert!(rejects(valid, 1, &"x".repeat(4097), "body"));
        assert!(rejects(valid, 1, "title", &"x".repeat(16385)));
    }

    #[test]
    fn clicks_reach_the_router_only_while_one_is_registered() {
        // `HANDLER` is process-wide; the native app test also sets it.
        let _serial = super::super::native::wayland_activation::tests::TOKEN_TESTS.lock();
        let (sent, received) = std::sync::mpsc::channel();
        set_notification_handler(Some(Arc::new(move |origin, pane| sent.send((origin, pane)).unwrap())));
        deliver_notification_click("o".into(), 4);
        assert_eq!(received.try_recv().unwrap(), ("o".to_owned(), 4));
        set_notification_handler(None);
        deliver_notification_click("o".into(), 5);
        assert!(received.try_recv().is_err());
    }
}
