//! Data the platform layer takes and reports, so it needs no other crate module.
//! The crate-root modules (`config`, `keymap`, `event`) convert to and from these.

use std::sync::Arc;

/// Window color space (`window-colorspace`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    Srgb,
    DisplayP3,
}

/// What the OS needs to know to style a window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowLook {
    pub colorspace: ColorSpace,
    /// Terminal background as `0xRRGGBBAA`.
    pub background: u32,
    pub background_opacity: f32,
    pub background_blur: u32,
}

/// Modifier keys of a global hotkey.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HotkeyMods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub sup: bool,
}

/// A system-wide hotkey in the platform's own terms: a macOS virtual key code
/// or an X11 keysym.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    pub code: u32,
    pub mods: HotkeyMods,
}

/// An entry of the macOS menu bar that Litty handles itself. The menu item's
/// tag indexes [`MenuCommand::ALL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    Quit,
    ToggleSecureInput,
    NewWindow,
    NewTab,
    SplitRight,
    SplitDown,
    CloseTab,
    Copy,
    Paste,
    SelectAll,
    Find,
    ClearScreen,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    OpenConfig,
    CheckUpdate,
}

impl MenuCommand {
    pub const ALL: [MenuCommand; 17] = [
        Self::Quit,
        Self::ToggleSecureInput,
        Self::NewWindow,
        Self::NewTab,
        Self::SplitRight,
        Self::SplitDown,
        Self::CloseTab,
        Self::Copy,
        Self::Paste,
        Self::SelectAll,
        Self::Find,
        Self::ClearScreen,
        Self::ZoomIn,
        Self::ZoomOut,
        Self::ZoomReset,
        Self::OpenConfig,
        Self::CheckUpdate,
    ];
}

/// Something the OS did that the application must react to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformEvent {
    /// Dock icon clicked or application activated.
    Reopen,
    /// The system appearance changed.
    SystemAppearance { dark: bool },
    Menu(MenuCommand),
    /// Quick Terminal hotkey or control-socket request.
    ToggleQuickTerminal,
}

/// Where platform callbacks (any thread) deliver [`PlatformEvent`]s.
pub type EventSink = Arc<dyn Fn(PlatformEvent) + Send + Sync>;

/// A slot holding the process-wide sink, shared by a platform's modules.
pub struct SinkSlot(parking_lot::Mutex<Option<EventSink>>);

impl SinkSlot {
    pub const fn new() -> Self {
        Self(parking_lot::const_mutex(None))
    }

    pub fn set(&self, sink: EventSink) {
        *self.0.lock() = Some(sink);
    }

    /// Deliver `event`; the callback runs without the slot locked.
    pub fn emit(&self, event: PlatformEvent) {
        let sink = self.0.lock().clone();
        if let Some(sink) = sink {
            sink(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn slot_delivers_to_the_latest_sink_and_ignores_events_without_one() {
        let slot = SinkSlot::new();
        slot.emit(PlatformEvent::Reopen);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        slot.set(Arc::new(move |e| log.lock().unwrap().push(e)));
        slot.emit(PlatformEvent::SystemAppearance { dark: true });
        slot.emit(PlatformEvent::Menu(MenuCommand::Quit));
        assert_eq!(
            *seen.lock().unwrap(),
            [PlatformEvent::SystemAppearance { dark: true }, PlatformEvent::Menu(MenuCommand::Quit)]
        );
    }

    #[test]
    fn menu_commands_are_listed_once_in_declaration_order() {
        for (i, c) in MenuCommand::ALL.iter().enumerate() {
            assert_eq!(MenuCommand::ALL.iter().position(|x| x == c), Some(i), "{c:?} listed twice");
        }
    }

    #[test]
    fn emit_does_not_hold_the_slot_lock() {
        // A callback that re-enters the slot must not deadlock.
        let slot: Arc<SinkSlot> = Arc::new(SinkSlot::new());
        let inner = slot.clone();
        slot.set(Arc::new(move |_| inner.set(Arc::new(|_| {}))));
        slot.emit(PlatformEvent::Reopen);
    }
}
