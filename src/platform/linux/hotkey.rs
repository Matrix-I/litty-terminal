//! Quick Terminal global hotkey on Linux.
//!
//! - X11: a passive key grab on the root window, served by a thread that
//!   waits for the key press.
//! - Wayland has no global hotkey API for clients, so the control socket
//!   (`platform::control_socket`) lets a desktop-environment shortcut run
//!   `litty +toggle-quick-terminal`. The socket works under X11 too.

use crate::platform::{EventSink, Hotkey};

/// Serve the control socket; it also provides the event sink that X11
/// hotkey presses use, so call it before [`set_global_hotkey`].
pub fn listen_for_toggle(sink: EventSink) {
    crate::platform::control_socket::listen(sink);
}

/// Grab (or, with `None`, release) the Quick Terminal hotkey on X11. Needs
/// [`listen_for_toggle`] first, which provides the event sink.
pub fn set_global_hotkey(hotkey: Option<Hotkey>) {
    x11::set(hotkey);
}

mod x11 {
    use std::sync::Arc;

    use parking_lot::Mutex;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask};
    use x11rb::protocol::Event;
    use x11rb::rust_connection::RustConnection;

    use crate::platform::Hotkey;

    struct Grabber {
        conn: Arc<RustConnection>,
        root: u32,
        /// Key code and modifiers currently grabbed.
        grabbed: Option<(u8, ModMask)>,
    }

    static GRABBER: Mutex<Option<Grabber>> = parking_lot::const_mutex(None);

    pub fn set(hotkey: Option<Hotkey>) {
        let mut slot = GRABBER.lock();
        if slot.is_none() {
            if hotkey.is_none() {
                return;
            }
            // No X server (pure Wayland): the control socket is the way in.
            let Ok((conn, screen)) = x11rb::connect(None) else {
                eprintln!(
                    "litty: no X11 display for the Quick Terminal hotkey; bind `litty +toggle-quick-terminal` to a desktop shortcut instead"
                );
                return;
            };
            let root = conn.setup().roots[screen].root;
            let conn = Arc::new(conn);
            let events = conn.clone();
            let spawned = std::thread::Builder::new().name("litty-hotkey".into()).spawn(move || {
                while let Ok(event) = events.wait_for_event() {
                    if let Event::KeyPress(_) = event {
                        crate::platform::control_socket::toggle();
                    }
                }
            });
            if spawned.is_err() {
                return;
            }
            *slot = Some(Grabber { conn, root, grabbed: None });
        }
        let Some(g) = slot.as_mut() else { return };
        if let Some((code, mods)) = g.grabbed.take() {
            for extra in lock_variants() {
                let _ = g.conn.ungrab_key(code, g.root, mods | extra);
            }
        }
        if let Some(h) = hotkey {
            match grab(g, h) {
                Ok(grabbed) => g.grabbed = Some(grabbed),
                Err(e) => eprintln!("litty: Quick Terminal hotkey: {e}"),
            }
        }
        let _ = g.conn.flush();
    }

    /// Caps Lock and Num Lock must not stop the hotkey: grab every combination.
    fn lock_variants() -> [ModMask; 4] {
        [ModMask::default(), ModMask::LOCK, ModMask::M2, ModMask::LOCK | ModMask::M2]
    }

    fn grab(g: &Grabber, t: Hotkey) -> Result<(u8, ModMask), String> {
        let sym = t.code;
        let setup = g.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = g
            .conn
            .get_keyboard_mapping(min, max - min + 1)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        let per = map.keysyms_per_keycode.max(1) as usize;
        let index = map.keysyms.chunks(per).position(|syms| syms.contains(&sym)).ok_or("key not on this keyboard")?;
        let code = min + index as u8;
        let m = t.mods;
        let mut mods = ModMask::default();
        for (on, mask) in [(m.shift, ModMask::SHIFT), (m.ctrl, ModMask::CONTROL), (m.alt, ModMask::M1), (m.sup, ModMask::M4)] {
            if on {
                mods |= mask;
            }
        }
        for extra in lock_variants() {
            g.conn
                .grab_key(true, g.root, mods | extra, code, GrabMode::ASYNC, GrabMode::ASYNC)
                .map_err(|e| e.to_string())?
                .check()
                .map_err(|_| "the key is taken by another application".to_string())?;
        }
        Ok((code, mods))
    }
}
