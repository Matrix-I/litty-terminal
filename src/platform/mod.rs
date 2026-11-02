//! OS integration that does not depend on window or terminal state:
//! detached child processes, notifications, URL / file opening, clipboard.

pub(crate) mod clipboard;
pub(crate) mod control_socket;
pub(crate) mod macos;
pub(crate) mod process;
