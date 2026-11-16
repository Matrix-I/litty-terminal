//! Native actionable notifications are macOS-only; Linux keeps its helper.
use std::{io, sync::Arc};
#[allow(dead_code)] // macOS callback registration has no Linux caller.
pub fn set_notification_handler(_: Option<Arc<dyn Fn(String, u64) + Send + Sync>>) {}
#[allow(dead_code)] // Native launch initialization is macOS-only.
pub fn init_notifications() -> io::Result<()> { Ok(()) }
#[allow(dead_code)] // Native notification cleanup is macOS-only.
pub fn shutdown_notifications() {}
/// Linux helper notifications never relaunch the app on click.
pub fn startup_is_notification() -> bool { false }
#[allow(dead_code)] // Linux has no AppKit reopen callback.
pub fn consume_notification_reopen() -> bool { false }
#[allow(dead_code)] // Matching API; existing Linux callers retain helper delivery.
pub fn show_pane_notification(_: &str, _: u64, title: &str, body: &str) -> io::Result<()> {
    crate::platform::process::show_notification(title, body);
    Ok(())
}
