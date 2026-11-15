//! macOS instance reuse has no Linux implementation in this phase.
use crate::platform::activation::{ActivationRequest, ActivationSink};

pub struct ActivationServer;
impl ActivationServer {
    pub fn origin(&self) -> &str {
        ""
    }
    pub fn accepts(&self, _: &ActivationRequest) -> bool {
        false
    }
}
pub fn start_activation(_: ActivationSink) -> std::io::Result<ActivationServer> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "instance activation is macOS-only",
    ))
}
pub fn reuse_window(_: &str) -> std::io::Result<bool> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "instance reuse is macOS-only",
    ))
}

pub struct NotificationRouter;
pub fn start_notification_router() -> std::io::Result<NotificationRouter> {
    Ok(NotificationRouter)
}
pub fn notify_pane(_: Option<&str>, _: u64, title: &str, body: &str) {
    crate::platform::process::show_notification(title, body);
}
