//! Seat-local text-input-v3. The PTY owns surrounding text, so we do not
//! advertise surrounding-text support or emulate deletion with backspaces.
use std::sync::{Arc, Weak};
use smithay_client_toolkit::reexports::client::{delegate_noop, Connection, Dispatch, QueueHandle};
use smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat;
use smithay_client_toolkit::reexports::protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3::ZwpTextInputManagerV3,
    zwp_text_input_v3::{self, ZwpTextInputV3, ContentHint, ContentPurpose},
};
use super::wayland::{State, WindowShared};
use crate::platform::gui::{Ime, WindowEvent};

pub(super) type Manager = ZwpTextInputManagerV3;

#[derive(Default)]
struct Pending {
    preedit: String,
    commit: Option<String>,
}
impl Pending {
    fn apply(&mut self, out: &mut Vec<WindowEvent>) {
        out.push(WindowEvent::Ime(Ime::Preedit(String::new())));
        if let Some(text) = self.commit.take().filter(|s| !s.is_empty()) {
            out.push(WindowEvent::Ime(Ime::Commit(text)));
        }
        out.push(WindowEvent::Ime(Ime::Preedit(std::mem::take(&mut self.preedit))));
    }
}

pub(super) struct TextInput {
    wl: ZwpTextInputV3,
    seat: WlSeat,
    focus: Option<Weak<WindowShared>>,
    enabled: bool,
    rect: Option<(i32, i32, i32, i32)>,
    pending: Pending,
}

fn rectangle(position: crate::platform::gui::PhysicalPosition<f64>, size: crate::platform::gui::PhysicalSize<f64>, scale: f64) -> (i32, i32, i32, i32) {
    let x = (position.x / scale).floor();
    let y = (position.y / scale).floor();
    (x as i32, y as i32, ((position.x + size.width) / scale - x).ceil().max(1.0) as i32,
        ((position.y + size.height) / scale - y).ceil().max(1.0) as i32)
}

impl TextInput {
    pub(super) fn update(&mut self, win: &Arc<WindowShared>) {
        if !self.focus.as_ref().is_some_and(|w| w.as_ptr() == Arc::as_ptr(win)) { return; }
        let mut st = win.state.lock();
        let enabled = st.ime_allowed;
        let rect = rectangle(st.ime_rect.0, st.ime_rect.1, st.scale());
        let changed = enabled != self.enabled;
        if changed {
            self.pending = Pending::default();
            self.rect = None;
            if enabled {
                self.wl.enable();
                self.wl.set_content_type(ContentHint::None, ContentPurpose::Terminal);
                st.events.push(WindowEvent::Ime(Ime::Enabled));
            } else {
                self.wl.disable();
                st.events.push(WindowEvent::Ime(Ime::Disabled));
            }
            self.enabled = enabled;
        }
        let moved = enabled && self.rect != Some(rect);
        if moved {
            self.wl.set_cursor_rectangle(rect.0, rect.1, rect.2, rect.3);
            self.rect = Some(rect);
        }
        if changed || moved { self.wl.commit(); }
    }
}
impl Drop for TextInput {
    fn drop(&mut self) {
        if let Some(win) = self.focus.as_ref().and_then(Weak::upgrade) {
            win.state.lock().events.push(WindowEvent::Ime(Ime::Disabled));
        }
        self.wl.destroy();
    }
}
impl State {
    pub(super) fn add_text_input(&mut self, qh: &QueueHandle<Self>, seat: WlSeat) {
        if self.text_inputs.iter().any(|i| i.seat == seat) { return; }
        if let Some(manager) = &self.text_manager {
            self.text_inputs.push(TextInput { wl: manager.get_text_input(&seat, qh, ()), seat,
                focus: None, enabled: false, rect: None, pending: Pending::default() });
        }
    }
    pub(super) fn remove_text_input(&mut self, seat: &WlSeat) {
        self.text_inputs.retain(|i| i.seat != *seat);
    }
}
impl Dispatch<ZwpTextInputV3, ()> for State {
    fn event(state: &mut Self, object: &ZwpTextInputV3, event: zwp_text_input_v3::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let win = match &event {
            zwp_text_input_v3::Event::Enter { surface } => state.window(surface),
            _ => None,
        };
        let Some(input) = state.text_inputs.iter_mut().find(|i| i.wl == *object) else { return; };
        match event {
            zwp_text_input_v3::Event::Enter { .. } => {
                input.focus = win.as_ref().map(Arc::downgrade);
                input.enabled = false;
                input.rect = None;
                input.pending = Pending::default();
                if let Some(win) = win { input.update(&win); }
            }
            zwp_text_input_v3::Event::Leave { .. } => {
                if let Some(win) = input.focus.take().and_then(|w| w.upgrade()) {
                    win.state.lock().events.push(WindowEvent::Ime(Ime::Disabled));
                }
                input.enabled = false;
                input.rect = None;
                input.pending = Pending::default();
            }
            zwp_text_input_v3::Event::PreeditString { text, .. } if input.enabled => input.pending.preedit = text.unwrap_or_default(),
            zwp_text_input_v3::Event::CommitString { text } if input.enabled => input.pending.commit = text,
            // A stale serial still applies incoming edits, per v3. We do
            // not write any client state in response to done, so it cannot
            // roll back a newer enable or candidate rectangle.
            zwp_text_input_v3::Event::Done { .. } => {
                if input.enabled && let Some(win) = input.focus.as_ref().and_then(Weak::upgrade) {
                    input.pending.apply(&mut win.state.lock().events);
                } else { input.pending = Pending::default(); }
            }
            // We do not advertise surrounding text: a terminal cannot edit
            // the shell's line by byte offsets. Cursor selection within the
            // preedit is not represented by the shared Ime API either.
            _ => {}
        }
    }
}
delegate_noop!(State: ignore Manager);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::gui::{PhysicalPosition, PhysicalSize};
    #[test]
    fn candidate_rectangle_uses_logical_pixels_and_covers_the_cursor() {
        assert_eq!(rectangle(PhysicalPosition::new(16.0, 32.0), PhysicalSize::new(10.0, 20.0), 1.5), (10, 21, 8, 14));
    }
    #[test]
    fn done_clears_old_preedit_commits_then_inserts_new_preedit() {
        let mut pending = Pending { preedit: "â".into(), commit: Some("tiếng".into()) };
        let mut events = Vec::new();
        pending.apply(&mut events);
        assert!(matches!(&events[..], [WindowEvent::Ime(Ime::Preedit(a)), WindowEvent::Ime(Ime::Commit(b)), WindowEvent::Ime(Ime::Preedit(c))] if a.is_empty() && b == "tiếng" && c == "â"));
        events.clear();
        pending.apply(&mut events);
        assert!(events.iter().all(|e| matches!(e, WindowEvent::Ime(Ime::Preedit(s)) if s.is_empty())));
    }
}
