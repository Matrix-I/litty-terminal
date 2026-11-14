//! One-shot launcher tokens and asynchronous requests for compositor-approved
//! activation. Tokens are never propagated to the terminal's child processes.
use std::sync::{OnceLock, Weak};
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use smithay_client_toolkit::reexports::client::protocol::{wl_seat::WlSeat, wl_surface::WlSurface};
use smithay_client_toolkit::reexports::protocols::xdg::activation::v1::client::{
    xdg_activation_v1::XdgActivationV1,
    xdg_activation_token_v1::{self, XdgActivationTokenV1},
};
use super::wayland::{State, WindowShared};

static STARTUP: OnceLock<Mutex<Option<String>>> = OnceLock::new();

/// Called from main before starting any threads. The unsafe environment
/// mutation is confined to that single-threaded startup boundary.
pub fn capture_startup_activation() {
    STARTUP.get_or_init(|| {
        let token = std::env::var("XDG_ACTIVATION_TOKEN").ok().filter(|s| valid_token(s));
        // SAFETY: main calls this before any worker/PTY/GUI threads exist.
        unsafe { std::env::remove_var("XDG_ACTIVATION_TOKEN") };
        Mutex::new(token)
    });
}
pub(super) fn startup_token() -> Option<String> { STARTUP.get()?.lock().take() }
fn valid_token(token: &str) -> bool { !token.is_empty() && token.len() <= 4096 && !token.contains('\0') }

pub(super) struct InputGrant {
    seat: WlSeat,
    surface: WlSurface,
    serial: u32,
    at: Instant,
}
impl InputGrant {
    pub(super) fn new(seat: WlSeat, surface: WlSurface, serial: u32) -> Self {
        Self { seat, surface, serial, at: Instant::now() }
    }
    fn fresh(&self) -> bool { self.at.elapsed() <= Duration::from_secs(5) && self.surface.is_alive() && self.seat.is_alive() }
}

struct Pending(XdgActivationTokenV1);
impl Drop for Pending {
    fn drop(&mut self) { self.0.destroy(); }
}
#[derive(Default)]
pub(super) struct Activation {
    pending: Option<Pending>,
}
pub(super) struct Request {
    window: Weak<WindowShared>,
    target: WlSurface,
}
impl Activation {
    pub(super) fn cancel(&mut self) { self.pending = None; }
    pub(super) fn request(
        &mut self, manager: &XdgActivationV1, qh: &QueueHandle<State>,
        target: &WlSurface, window: Weak<WindowShared>, grant: Option<&InputGrant>,
    ) {
        if self.pending.is_some() { return; }
        let token = manager.get_activation_token(qh, Request { window, target: target.clone() });
        token.set_app_id("litty".into());
        if let Some(input) = grant.filter(|input| input.fresh()) {
            token.set_serial(input.serial, &input.seat);
            token.set_surface(&input.surface);
        }
        token.commit();
        self.pending = Some(Pending(token));
    }
}
impl Dispatch<XdgActivationTokenV1, Request> for State {
    fn event(state: &mut Self, proxy: &XdgActivationTokenV1, event: xdg_activation_token_v1::Event, request: &Request, _: &Connection, _: &QueueHandle<Self>) {
        let xdg_activation_token_v1::Event::Done { token } = event else { return; };
        let Some(win) = request.window.upgrade() else { return; };
        let mut st = win.state.lock();
        if !st.activation.pending.as_ref().is_some_and(|p| p.0 == *proxy) { return; }
        st.activation.cancel();
        if valid_token(&token) && request.target.is_alive()
            && let Some(manager) = &state.activation_manager
        { manager.activate(token, &request.target); }
    }
}
delegate_noop!(State: ignore XdgActivationV1);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launcher_tokens_are_bounded_opaque_values() {
        assert!(valid_token("opaque_token/with:punctuation"));
        assert!(!valid_token(""));
        assert!(!valid_token("a\0b"));
        assert!(!valid_token(&"a".repeat(4097)));
    }
}
