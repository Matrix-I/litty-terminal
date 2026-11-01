//! Terminal modes set by the application (DECSET / DECRST, IRM, DECSCUSR,
//! XTMODKEYS, Kitty keyboard, DEC 2027), reset together by RIS.

use super::*;

/// `grapheme-width-method = unicode`: new terminals start in DEC mode 2027.
pub(super) static GRAPHEME_DEFAULT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Set the initial state of DEC mode 2027 (grapheme clustering) for new terminals.
pub fn set_grapheme_default(on: bool) {
    GRAPHEME_DEFAULT.store(on, Ordering::Relaxed);
}

/// Mode flags of one terminal.
pub struct TerminalModes {
    pub cursor_visible: bool,
    /// Last DECSCUSR (`CSI Ps SP q`) value: 0 = config default,
    /// 1/2 = blinking/steady block, 3/4 = underline, 5/6 = bar.
    pub cursor_shape: u8,
    pub app_cursor: bool,
    pub bracketed_paste: bool,
    pub sync_output: bool,
    /// When synchronized output (DECSET 2026) began; rendering is deferred meanwhile.
    pub sync_started: Option<std::time::Instant>,
    pub(super) autowrap: bool,
    /// IRM: printing shifts the rest of the line right instead of overwriting.
    pub(super) insert_mode: bool,
    pub mouse_tracking: MouseTracking,
    /// SGR mouse encoding (DECSET 1006).
    pub mouse_sgr: bool,
    /// Report focus in/out (DECSET 1004).
    pub focus_events: bool,
    /// Translate the mouse wheel into arrow keys on the alternate screen (DECSET 1007).
    pub alt_scroll: bool,
    /// SGR mouse reports carry pixel instead of cell coordinates (DECSET 1016).
    pub mouse_sgr_pixels: bool,
    /// Report system light/dark appearance changes (DECSET 2031).
    pub color_scheme_reports: bool,
    /// xterm `modifyOtherKeys` level (0 off, 1, 2) set by `CSI > 4 ; level m`.
    pub modify_other_keys: u8,
    /// Kitty keyboard protocol state for the main and the alternate screen.
    pub(super) kitty_keyboard: [KittyKeyboard; 2],
    /// DEC mode 2027: emoji sequences (ZWJ, flags, skin tones, VS16) share one cell pair.
    pub(super) grapheme_mode: bool,
}

impl Grid {
    /// Active Kitty keyboard protocol flags (`keymap::KITTY_*`) of the current screen.
    pub fn kitty_keyboard_flags(&self) -> u8 {
        self.modes.kitty_keyboard[self.alt_active as usize].flags
    }
}
