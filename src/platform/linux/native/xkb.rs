//! Keyboard layouts for Wayland windows through libxkbcommon, opened at run
//! time (`xkbcommon-dl`) like libX11 and libwayland, so the binary links
//! nothing but libc. Without the library, keys follow the built-in evdev
//! table (a US layout).

use std::ffi::{CStr, c_char};
use std::os::fd::{AsRawFd, OwnedFd};

use xkbcommon_dl::{
    XKB_MOD_NAME_ALT, XKB_MOD_NAME_CTRL, XKB_MOD_NAME_LOGO, XKB_MOD_NAME_SHIFT, XkbCommon,
    xkb_context_flags, xkb_keymap, xkb_keymap_compile_flags, xkb_keymap_format, xkb_keysym_t, xkb_mod_index_t,
    xkb_state, xkb_state_component,
};

use super::events::{convert_linux_key, mods_from_state};
use crate::platform::input::{Key, KeyInput, KeyLocation, Mods, NamedKey};

/// Wayland key codes are evdev codes; xkb numbers keys as X11 does, 8 higher.
const EVDEV_OFFSET: u32 = 8;

/// A compiled keymap and its modifier and layout state.
pub struct Keymap {
    xkb: &'static XkbCommon,
    keymap: *mut xkb_keymap,
    state: *mut xkb_state,
    shift: xkb_mod_index_t,
    ctrl: xkb_mod_index_t,
    alt: xkb_mod_index_t,
    logo: xkb_mod_index_t,
    compose: Option<Compose>,
}

// The pointers are owned by this value alone and libxkbcommon keeps no
// thread-local state; the dispatch lock serializes every use.
unsafe impl Send for Keymap {}

/// libxkbcommon, or `None` (reported once) when it is not installed.
fn library() -> Option<&'static XkbCommon> {
    let lib = xkbcommon_dl::xkbcommon_option();
    if lib.is_none() {
        static WARNED: std::sync::Once = std::sync::Once::new();
        WARNED.call_once(|| eprintln!("litty: libxkbcommon is not installed; Wayland keys use a US layout"));
    }
    lib
}

impl Keymap {
    /// The keymap a compositor sends (`wl_keyboard.keymap`, format xkb v1):
    /// `size` bytes of NUL-terminated text behind `fd`. `Ok(None)` without
    /// libxkbcommon (reported once already).
    pub fn from_fd(fd: &OwnedFd, size: u32) -> Result<Option<Self>, String> {
        let Some(xkb) = library() else { return Ok(None) };
        let len = size as usize;
        if len == 0 {
            return Err("the compositor sent an empty keymap".into());
        }
        // Reading past the file's end would kill the process (SIGBUS).
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd.as_raw_fd(), &mut stat) } != 0 || (stat.st_size as u64) < len as u64 {
            return Err(format!("the compositor's keymap is shorter than the {len} bytes it announced"));
        }
        // Private, read-only: the compositor may share one mapping with every client.
        let map = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, fd.as_raw_fd(), 0) };
        if map == libc::MAP_FAILED {
            return Err(format!("cannot map the keymap: {}", std::io::Error::last_os_error()));
        }
        let text = unsafe { std::slice::from_raw_parts(map.cast::<u8>(), len) };
        let text = &text[..text.iter().position(|&b| b == 0).unwrap_or(len)];
        let keymap = Self::compile(xkb, |ctx| unsafe {
            (xkb.xkb_keymap_new_from_buffer)(
                ctx,
                text.as_ptr().cast(),
                text.len(),
                xkb_keymap_format::XKB_KEYMAP_FORMAT_TEXT_V1,
                xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS,
            )
        });
        unsafe { libc::munmap(map, len) };
        keymap.map(Some)
    }

    /// A keymap from RMLVO names (`layout` such as `us`, `options` such as
    /// `caps:escape`), for tests.
    #[cfg(test)]
    pub fn from_names(layout: &CStr, options: &CStr) -> Result<Self, String> {
        let xkb = library().ok_or("libxkbcommon is not installed")?;
        let names = xkbcommon_dl::xkb_rule_names {
            rules: std::ptr::null(),
            model: std::ptr::null(),
            layout: layout.as_ptr(),
            variant: std::ptr::null(),
            options: options.as_ptr(),
        };
        Self::compile(xkb, |ctx| unsafe {
            (xkb.xkb_keymap_new_from_names)(ctx, &names, xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS)
        })
    }

    fn compile(xkb: &'static XkbCommon, new: impl FnOnce(*mut xkbcommon_dl::xkb_context) -> *mut xkb_keymap) -> Result<Self, String> {
        let ctx = unsafe { (xkb.xkb_context_new)(xkb_context_flags::XKB_CONTEXT_NO_FLAGS) };
        if ctx.is_null() {
            return Err("cannot create an xkb context".into());
        }
        // The keymap keeps its own reference to the context.
        let keymap = new(ctx);
        let compose = Compose::new(ctx);
        unsafe { (xkb.xkb_context_unref)(ctx) };
        if keymap.is_null() {
            return Err("cannot compile the keymap".into());
        }
        let state = unsafe { (xkb.xkb_state_new)(keymap) };
        if state.is_null() {
            unsafe { (xkb.xkb_keymap_unref)(keymap) };
            return Err("cannot create the keyboard state".into());
        }
        let index = |name: &[u8]| unsafe { (xkb.xkb_keymap_mod_get_index)(keymap, name.as_ptr().cast()) };
        Ok(Self {
            xkb,
            keymap,
            state,
            shift: index(XKB_MOD_NAME_SHIFT),
            ctrl: index(XKB_MOD_NAME_CTRL),
            alt: index(XKB_MOD_NAME_ALT),
            logo: index(XKB_MOD_NAME_LOGO),
            compose,
        })
    }

    pub fn reset_compose(&mut self) {
        if let Some(compose) = &mut self.compose { compose.reset(); }
    }

    /// `Some` consumes a composing press; an empty string is an unfinished
    /// or cancelled sequence. Shortcuts bypass Compose.
    pub fn compose(&mut self, evdev: u32) -> Option<String> {
        let mods = self.mods();
        if mods.ctrl || mods.alt || mods.sup { self.reset_compose(); return None; }
        let sym = unsafe { (self.xkb.xkb_state_key_get_one_sym)(self.state, evdev.saturating_add(EVDEV_OFFSET)) };
        self.compose.as_mut()?.feed(sym)
    }

    /// Apply `wl_keyboard.modifiers`.
    pub fn update_mask(&mut self, depressed: u32, latched: u32, locked: u32, group: u32) {
        unsafe { (self.xkb.xkb_state_update_mask)(self.state, depressed, latched, locked, 0, 0, group) };
    }

    fn active(&self, index: xkb_mod_index_t) -> bool {
        // An index the keymap lacks (XKB_MOD_INVALID) reads as -1, inactive.
        unsafe {
            (self.xkb.xkb_state_mod_index_is_active)(self.state, index, xkb_state_component::XKB_STATE_MODS_EFFECTIVE) > 0
        }
    }

    pub fn mods(&self) -> Mods {
        Mods { ctrl: self.active(self.ctrl), alt: self.active(self.alt), shift: self.active(self.shift), sup: self.active(self.logo) }
    }

    /// Whether holding the key repeats it (modifiers and locks do not).
    pub fn repeats(&self, evdev: u32) -> bool {
        unsafe { (self.xkb.xkb_keymap_key_repeats)(self.keymap, evdev.saturating_add(EVDEV_OFFSET)) != 0 }
    }

    /// Text of a keysym, without control characters (Ctrl is the keymap's to apply).
    fn text(&self, sym: xkb_keysym_t) -> Option<String> {
        let mut buf = [0 as c_char; 8];
        // Bytes written including the NUL; 0 when the keysym has no text.
        let n = unsafe { (self.xkb.xkb_keysym_to_utf8)(sym, buf.as_mut_ptr(), buf.len()) };
        if n <= 1 {
            return None;
        }
        let text = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_str().ok()?;
        (!text.chars().any(char::is_control)).then(|| text.to_owned())
    }

    /// The keysym at the key's first level in the first layout: the key
    /// without Shift or AltGr, in the layout shortcuts are written for (X11
    /// looks the base key up in group 1 too), so `ctrl+shift+c` matches
    /// whichever layout is active.
    fn base_sym(&self, keycode: u32) -> Option<xkb_keysym_t> {
        let mut syms: *const xkb_keysym_t = std::ptr::null();
        let n = unsafe { (self.xkb.xkb_keymap_key_get_syms_by_level)(self.keymap, keycode, 0, 0, &mut syms) };
        (n == 1 && !syms.is_null()).then(|| unsafe { *syms })
    }

    /// The key event for an evdev code under the current state. The evdev
    /// table names the physical key and its place (keypad or not); the
    /// keysym the keymap gives decides the logical key, so remapped keys
    /// (Caps Lock as Escape) and generated keymaps (virtual keyboards) mean
    /// what they produce, not what the key is labelled.
    pub fn key(&self, evdev: u32, pressed: bool, repeat: bool) -> KeyInput {
        // Codes past the keymap's range just look up nothing.
        let keycode = evdev.saturating_add(EVDEV_OFFSET);
        let mut key = convert_linux_key(evdev, 0, pressed, repeat);
        // Shift, Caps Lock, AltGr and Num Lock pick the level; Ctrl, Alt
        // and Super select none in standard layouts, so the text stays the
        // printable character the keymap matches against.
        let sym = unsafe { (self.xkb.xkb_state_key_get_one_sym)(self.state, keycode) };
        if let Some((name, location)) = named_key(sym) {
            key.logical = Key::Named(name);
            key.unshifted = Key::Named(name);
            key.text = None;
            key.location = location.unwrap_or(key.location);
            return key;
        }
        if let Some(text) = self.text(sym) {
            key.logical = Key::Character(text.clone());
            key.text = Some(text);
        } else {
            // Dead keys and `NoSymbol`: the evdev table's US character is
            // not what this layout puts there, so the key types nothing.
            key.logical = Key::Unidentified;
            key.text = None;
        }
        // A keypad key's base key is its Num Lock one (`KP_7`, not
        // `KP_Home`), as on X11; any other key's is its first level.
        let base = if key.location == KeyLocation::Numpad { Some(sym) } else { self.base_sym(keycode) };
        if let Some(text) = base.and_then(|sym| self.text(sym)) {
            key.unshifted = Key::Character(text);
        } else if key.location == KeyLocation::Numpad {
            key.unshifted = key.logical.clone();
        }
        key
    }
}

/// Owned Compose state; the dispatch lock also serializes its raw pointer.
struct Compose {
    lib: &'static xkbcommon_dl::XkbCommonCompose,
    state: *mut xkbcommon_dl::xkb_compose_state,
}
impl Compose {
    fn new(ctx: *mut xkbcommon_dl::xkb_context) -> Option<Self> {
        use xkbcommon_dl::{xkb_compose_compile_flags as C, xkb_compose_state_flags as S};
        let lib = xkbcommon_dl::xkbcommon_compose_option()?;
        let locale = ["LC_ALL", "LC_CTYPE", "LANG"].iter().find_map(|name| std::env::var(name).ok().filter(|s| !s.is_empty())).unwrap_or_else(|| "C".into());
        let locale = std::ffi::CString::new(locale).ok()?;
        let table = unsafe { (lib.xkb_compose_table_new_from_locale)(ctx, locale.as_ptr(), C::XKB_COMPOSE_COMPILE_NO_FLAGS) };
        if table.is_null() { return None; }
        let state = unsafe { (lib.xkb_compose_state_new)(table, S::XKB_COMPOSE_STATE_NO_FLAGS) };
        unsafe { (lib.xkb_compose_table_unref)(table) };
        (!state.is_null()).then_some(Self { lib, state })
    }
    fn reset(&mut self) { unsafe { (self.lib.xkb_compose_state_reset)(self.state) }; }
    fn feed(&mut self, sym: xkb_keysym_t) -> Option<String> {
        use xkbcommon_dl::xkb_compose_status::*;
        unsafe { (self.lib.xkb_compose_state_feed)(self.state, sym) };
        match unsafe { (self.lib.xkb_compose_state_get_status)(self.state) } {
            XKB_COMPOSE_NOTHING => None,
            XKB_COMPOSE_COMPOSING => Some(String::new()),
            XKB_COMPOSE_CANCELLED => { self.reset(); Some(String::new()) }
            XKB_COMPOSE_COMPOSED => {
                let len = unsafe { (self.lib.xkb_compose_state_get_utf8)(self.state, std::ptr::null_mut(), 0) }.max(0) as usize;
                let mut buf = vec![0u8; len + 1];
                unsafe { (self.lib.xkb_compose_state_get_utf8)(self.state, buf.as_mut_ptr().cast(), buf.len()) };
                buf.truncate(len);
                self.reset();
                Some(String::from_utf8(buf).unwrap_or_default())
            }
        }
    }
}
impl Drop for Compose {
    fn drop(&mut self) { unsafe { (self.lib.xkb_compose_state_unref)(self.state) }; }
}

/// The named key a keysym stands for, and the side for modifiers; `None`
/// for keysyms that type text (or that Litty has no name for).
fn named_key(sym: xkb_keysym_t) -> Option<(NamedKey, Option<KeyLocation>)> {
    use xkbcommon_dl::keysyms as k;
    let (left, right, numpad) = (Some(KeyLocation::Left), Some(KeyLocation::Right), Some(KeyLocation::Numpad));
    let named = |n| Some((n, None));
    match sym {
        k::Return => named(NamedKey::Enter),
        k::KP_Enter => Some((NamedKey::Enter, numpad)),
        k::Tab | k::ISO_Left_Tab | k::KP_Tab => named(NamedKey::Tab),
        k::space => named(NamedKey::Space),
        k::BackSpace => named(NamedKey::Backspace),
        k::Escape => named(NamedKey::Escape),
        k::Delete | k::KP_Delete => named(NamedKey::Delete),
        k::Insert | k::KP_Insert => named(NamedKey::Insert),
        k::Home | k::KP_Home => named(NamedKey::Home),
        k::End | k::KP_End => named(NamedKey::End),
        k::Prior | k::KP_Prior => named(NamedKey::PageUp),
        k::Next | k::KP_Next => named(NamedKey::PageDown),
        k::Up | k::KP_Up => named(NamedKey::ArrowUp),
        k::Down | k::KP_Down => named(NamedKey::ArrowDown),
        k::Left | k::KP_Left => named(NamedKey::ArrowLeft),
        k::Right | k::KP_Right => named(NamedKey::ArrowRight),
        k::KP_Begin | k::Clear => named(NamedKey::Clear),
        k::F1..=k::F20 => {
            const F: [NamedKey; 20] = [
                NamedKey::F1, NamedKey::F2, NamedKey::F3, NamedKey::F4, NamedKey::F5, NamedKey::F6, NamedKey::F7,
                NamedKey::F8, NamedKey::F9, NamedKey::F10, NamedKey::F11, NamedKey::F12, NamedKey::F13, NamedKey::F14,
                NamedKey::F15, NamedKey::F16, NamedKey::F17, NamedKey::F18, NamedKey::F19, NamedKey::F20,
            ];
            named(F[(sym - k::F1) as usize])
        }
        k::Shift_L => Some((NamedKey::Shift, left)),
        k::Shift_R => Some((NamedKey::Shift, right)),
        k::Control_L => Some((NamedKey::Control, left)),
        k::Control_R => Some((NamedKey::Control, right)),
        k::Alt_L => Some((NamedKey::Alt, left)),
        k::Alt_R => Some((NamedKey::Alt, right)),
        k::Super_L => Some((NamedKey::Super, left)),
        k::Super_R => Some((NamedKey::Super, right)),
        k::Meta_L => Some((NamedKey::Meta, left)),
        k::Meta_R => Some((NamedKey::Meta, right)),
        k::Hyper_L => Some((NamedKey::Hyper, left)),
        k::Hyper_R => Some((NamedKey::Hyper, right)),
        k::Caps_Lock => named(NamedKey::CapsLock),
        k::Num_Lock => named(NamedKey::NumLock),
        k::Scroll_Lock => named(NamedKey::ScrollLock),
        k::Print => named(NamedKey::PrintScreen),
        k::Pause => named(NamedKey::Pause),
        k::Menu => named(NamedKey::ContextMenu),
        k::XF86_AudioPlay => named(NamedKey::MediaPlay),
        k::XF86_AudioPause => named(NamedKey::MediaPause),
        k::XF86_AudioStop => named(NamedKey::MediaStop),
        k::XF86_AudioRecord => named(NamedKey::MediaRecord),
        k::XF86_AudioForward => named(NamedKey::MediaFastForward),
        k::XF86_AudioRewind => named(NamedKey::MediaRewind),
        k::XF86_AudioNext => named(NamedKey::MediaTrackNext),
        k::XF86_AudioPrev => named(NamedKey::MediaTrackPrevious),
        k::XF86_AudioLowerVolume => named(NamedKey::AudioVolumeDown),
        k::XF86_AudioRaiseVolume => named(NamedKey::AudioVolumeUp),
        k::XF86_AudioMute => named(NamedKey::AudioVolumeMute),
        _ => None,
    }
}

impl Drop for Keymap {
    fn drop(&mut self) {
        unsafe {
            (self.xkb.xkb_state_unref)(self.state);
            (self.xkb.xkb_keymap_unref)(self.keymap);
        }
    }
}

/// Modifiers from `wl_keyboard.modifiers` without a keymap to name them:
/// every common keymap puts the X11 core modifiers at their usual bits.
pub fn fallback_mods(depressed: u32, latched: u32, locked: u32) -> Mods {
    mods_from_state(depressed | latched | locked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::input::{KeyCode, NamedKey, PhysicalKey};

    #[test]
    fn dead_keys_and_compose_commit_unicode_and_reset() {
        use xkbcommon_dl::keysyms as k;
        let Some(mut keymap) = keymap(c"us") else { return };
        let compose = keymap.compose.as_mut().expect("Compose table for the current locale");
        assert_eq!(compose.feed(k::dead_acute), Some(String::new()));
        assert_eq!(compose.feed(k::e), Some("é".into()));
        assert_eq!(compose.feed(k::Multi_key), Some(String::new()));
        assert_eq!(compose.feed(k::o), Some(String::new()));
        assert_eq!(compose.feed(k::c), Some("©".into()));
        assert_eq!(compose.feed(k::a), None);
        compose.feed(k::dead_acute);
        compose.reset();
        assert_eq!(compose.feed(k::e), None, "focus loss cancels composition");
    }

    #[test]
    fn layout_dead_key_and_cancelled_sequence_do_not_leak_raw_text() {
        let Some(mut keymap) = keymap(c"de") else { return };
        assert_eq!(keymap.compose(13), Some(String::new()), "German acute key starts composition");
        assert_eq!(keymap.compose(18), Some("é".into()));
        assert_eq!(keymap.compose(13), Some(String::new()));
        assert_eq!(keymap.compose(1), Some(String::new()), "Escape cancels without typing");
        assert_eq!(keymap.compose(18), None, "next e uses the normal key path");
    }

    #[test]
    fn shortcuts_cancel_local_composition() {
        use xkbcommon_dl::keysyms as k;
        let Some(mut keymap) = keymap(c"us") else { return };
        keymap.compose.as_mut().unwrap().feed(k::dead_acute);
        keymap.update_mask(4, 0, 0, 0);
        assert_eq!(keymap.compose(46), None, "Ctrl+C is still a shell shortcut");
        keymap.update_mask(0, 0, 0, 0);
        assert_eq!(keymap.compose(18), None, "next e is not composed");
    }

    // evdev codes
    const KEY_Y: u32 = 21;
    const KEY_A: u32 = 30;
    const KEY_C: u32 = 46;
    const KEY_1: u32 = 2;
    const KEY_ENTER: u32 = 28;
    const KEY_LEFTSHIFT: u32 = 42;
    const KEY_KP7: u32 = 71;
    const KEY_CAPSLOCK: u32 = 58;
    const KEY_F1: u32 = 59;
    const KEY_F12: u32 = 88;
    const KEY_RIGHTCTRL: u32 = 97;
    const KEY_GRAVE: u32 = 41;

    // X11 core modifier bits, as standard keymaps number them.
    const SHIFT: u32 = 1;
    const CTRL: u32 = 4;
    const MOD2_NUM: u32 = 16;

    /// A keymap, or `None` (the test is skipped) without libxkbcommon or
    /// its layout data.
    fn keymap(layout: &CStr) -> Option<Keymap> {
        keymap_with(layout, c"")
    }

    fn keymap_with(layout: &CStr, options: &CStr) -> Option<Keymap> {
        match Keymap::from_names(layout, options) {
            Ok(k) => Some(k),
            Err(e) => {
                eprintln!("{e}; skipping");
                None
            }
        }
    }

    fn chars(key: &KeyInput) -> (Key, Key, Option<&str>) {
        (key.logical.clone(), key.unshifted.clone(), key.text.as_deref())
    }

    fn ch(s: &str) -> Key {
        Key::Character(s.into())
    }

    #[test]
    fn shift_picks_the_upper_level_and_keeps_the_base_key() {
        let Some(mut k) = keymap(c"us") else { return };
        assert_eq!(chars(&k.key(KEY_A, true, false)), (ch("a"), ch("a"), Some("a")));
        k.update_mask(SHIFT, 0, 0, 0);
        assert_eq!(chars(&k.key(KEY_A, true, false)), (ch("A"), ch("a"), Some("A")));
        assert_eq!(chars(&k.key(KEY_1, true, false)), (ch("!"), ch("1"), Some("!")));
        assert_eq!(k.mods(), Mods { shift: true, ..Mods::default() });
    }

    /// Ctrl+C must reach the keymap as `c` with Ctrl, not as the control
    /// character ETX: the keymap decides what Ctrl does.
    #[test]
    fn ctrl_leaves_the_printable_character() {
        let Some(mut k) = keymap(c"us") else { return };
        k.update_mask(CTRL, 0, 0, 0);
        let key = k.key(KEY_C, true, false);
        assert_eq!(chars(&key), (ch("c"), ch("c"), Some("c")));
        assert_eq!(k.mods(), Mods { ctrl: true, ..Mods::default() });
    }

    /// The layout names the character; the physical key stays the evdev one
    /// (shortcuts like Ctrl+Shift+C keep working on any layout).
    #[test]
    fn the_layout_decides_the_character() {
        let Some(k) = keymap(c"de") else { return };
        let key = k.key(KEY_Y, true, false);
        assert_eq!(chars(&key), (ch("z"), ch("z"), Some("z")));
        assert_eq!(key.physical, PhysicalKey::Code(KeyCode::KeyY));
    }

    /// With a second layout active the character follows it, but the base
    /// key stays the first layout's, as on X11: `ctrl+shift+c` keeps working.
    #[test]
    fn the_base_key_comes_from_the_first_layout() {
        let Some(mut k) = keymap(c"us,ru") else { return };
        k.update_mask(0, 0, 0, 1);
        let key = k.key(KEY_C, true, false);
        assert_eq!(chars(&key), (ch("с"), ch("c"), Some("с")));
    }

    /// A dead key before Compose, or a key mapped to nothing, types
    /// nothing, not the US character printed on that key.
    #[test]
    fn a_key_without_text_in_the_layout_types_nothing() {
        let Some(k) = keymap(c"de") else { return };
        let key = k.key(KEY_GRAVE, true, false);
        assert_eq!(key.text, None);
        assert_eq!(key.logical, Key::Unidentified);
    }

    #[test]
    fn special_keys_keep_their_names_and_carry_no_text() {
        let Some(k) = keymap(c"us") else { return };
        let enter = k.key(KEY_ENTER, true, false);
        assert_eq!((enter.logical, enter.text), (Key::Named(NamedKey::Enter), None));
    }

    /// The keypad follows Num Lock as on X11: digits with it, navigation
    /// keys without, and the digit is the base key either way.
    #[test]
    fn the_keypad_follows_num_lock() {
        let Some(mut k) = keymap(c"us") else { return };
        k.update_mask(0, 0, MOD2_NUM, 0);
        let kp7 = k.key(KEY_KP7, true, false);
        assert_eq!((kp7.logical, kp7.location), (ch("7"), KeyLocation::Numpad));
        k.update_mask(0, 0, 0, 0);
        assert_eq!(k.key(KEY_KP7, true, false).logical, Key::Named(NamedKey::Home));
    }

    /// What the key produces wins over its label: with Caps Lock remapped
    /// to Escape (`caps:escape`), the Caps Lock key is Escape.
    #[test]
    fn a_remapped_key_is_what_it_produces() {
        let Some(k) = keymap_with(c"us", c"caps:escape") else { return };
        let caps = k.key(KEY_CAPSLOCK, true, false);
        assert_eq!((caps.logical, caps.text), (Key::Named(NamedKey::Escape), None));
    }

    /// F-keys and modifiers carry their names (Kitty keyboard protocol
    /// encodes them), modifiers with their side.
    #[test]
    fn function_and_modifier_keys_are_named() {
        let Some(k) = keymap(c"us") else { return };
        assert_eq!(k.key(KEY_F1, true, false).logical, Key::Named(NamedKey::F1));
        assert_eq!(k.key(KEY_F12, true, false).logical, Key::Named(NamedKey::F12));
        let shift = k.key(KEY_LEFTSHIFT, true, false);
        assert_eq!((shift.logical, shift.location), (Key::Named(NamedKey::Shift), KeyLocation::Left));
        let ctrl = k.key(KEY_RIGHTCTRL, true, false);
        assert_eq!((ctrl.logical, ctrl.location), (Key::Named(NamedKey::Control), KeyLocation::Right));
    }

    #[test]
    fn modifiers_do_not_repeat_and_letters_do() {
        let Some(k) = keymap(c"us") else { return };
        assert!(k.repeats(KEY_A));
        assert!(!k.repeats(KEY_LEFTSHIFT));
    }

    #[test]
    fn press_release_and_repeat_flags_pass_through() {
        let Some(k) = keymap(c"us") else { return };
        let key = k.key(KEY_A, false, false);
        assert!(!key.pressed && !key.repeat);
        let key = k.key(KEY_A, true, true);
        assert!(key.pressed && key.repeat);
    }

    #[test]
    fn without_a_keymap_modifiers_use_the_core_bits() {
        assert_eq!(fallback_mods(SHIFT, 0, 0), Mods { shift: true, ..Mods::default() });
        assert_eq!(fallback_mods(0, CTRL, 0), Mods { ctrl: true, ..Mods::default() });
        assert_eq!(fallback_mods(0, 0, 8 | 64), Mods { alt: true, sup: true, ..Mods::default() });
    }

    /// A keymap the compositor sends arrives as a file descriptor.
    #[test]
    fn a_keymap_is_read_from_the_compositors_file() {
        let Some(k) = keymap(c"us") else { return };
        let raw = unsafe { (k.xkb.xkb_keymap_get_as_string)(k.keymap, xkb_keymap_format::XKB_KEYMAP_FORMAT_TEXT_V1) };
        assert!(!raw.is_null());
        let bytes = unsafe { CStr::from_ptr(raw) }.to_bytes_with_nul().to_vec();
        unsafe { libc::free(raw.cast_mut().cast()) };
        let file = tempfile_with(&bytes);
        let from_fd = Keymap::from_fd(&file, bytes.len() as u32).expect("keymap from fd").expect("libxkbcommon");
        assert_eq!(from_fd.key(KEY_A, true, false).text.as_deref(), Some("a"));
        assert!(Keymap::from_fd(&file, 0).is_err(), "an empty keymap is refused");
        assert!(Keymap::from_fd(&file, bytes.len() as u32 + 4096).is_err(), "a short file is refused, not read past");
    }

    fn tempfile_with(bytes: &[u8]) -> OwnedFd {
        use std::io::Write;
        use std::os::fd::FromRawFd;
        let fd = unsafe { libc::memfd_create(c"litty-keymap-test".as_ptr(), libc::MFD_CLOEXEC) };
        assert!(fd >= 0);
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        file.write_all(bytes).unwrap();
        file.into()
    }
}
