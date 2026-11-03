//! Application event loop wakeups and custom event types.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use winit::event_loop::EventLoopProxy;

use crate::keymap;
use crate::update;

#[derive(Debug, Clone)]
pub enum AppEvent {
    WakeUp,
    AiStatus { pane_id: u64, generation: u64, text: String },
    AiResult { pane_id: u64, generation: u64, outcome: Box<crate::ai::Outcome> },
    ChildExit(u64),
    Reopen,
    /// A menu bar item (macOS) was chosen.
    Menu(keymap::Action),
    /// The system switched between Light and Dark appearance (macOS).
    SystemAppearance { dark: bool },
    UpdateCheckResult(Result<update::ReleaseInfo, String>),
    UpdateInstallResult(Result<update::Version, String>),
    BenchAction(crate::bench::BenchCommand),
}

impl From<crate::platform::PlatformEvent> for AppEvent {
    fn from(event: crate::platform::PlatformEvent) -> Self {
        use crate::platform::{MenuCommand as M, PlatformEvent as P};
        use keymap::Action as A;
        match event {
            P::Reopen => AppEvent::Reopen,
            P::SystemAppearance { dark } => AppEvent::SystemAppearance { dark },
            P::ToggleQuickTerminal => AppEvent::Menu(A::ToggleQuickTerminal),
            P::Menu(command) => AppEvent::Menu(match command {
                M::Quit => A::Quit,
                M::ToggleSecureInput => A::ToggleSecureInput,
                M::NewWindow => A::NewWindow,
                M::NewTab => A::NewTab,
                M::SplitRight => A::SplitRight,
                M::SplitDown => A::SplitDown,
                M::CloseTab => A::CloseTab,
                M::Copy => A::Copy,
                M::Paste => A::Paste,
                M::SelectAll => A::SelectAll,
                M::Find => A::Find,
                M::ClearScreen => A::ClearScreen,
                M::ZoomIn => A::ZoomIn,
                M::ZoomOut => A::ZoomOut,
                M::ZoomReset => A::ZoomReset,
                M::OpenConfig => A::OpenConfig,
                M::CheckUpdate => A::CheckUpdate,
            }),
        }
    }
}

/// The sink platform callbacks use to post their events to the event loop.
pub fn platform_sink(proxy: EventLoopProxy<AppEvent>) -> crate::platform::EventSink {
    Arc::new(move |event| {
        let _ = proxy.send_event(event.into());
    })
}

/// Wakes the event loop from PTY reader threads, coalescing bursts of output
/// into a single pending `WakeUp`.
#[derive(Clone)]
pub struct Waker {
    pub proxy: EventLoopProxy<AppEvent>,
    pub queued: Arc<AtomicBool>,
}

impl Waker {
    pub fn new(proxy: EventLoopProxy<AppEvent>) -> Self {
        Waker {
            proxy,
            queued: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn wake(&self) {
        if !self.queued.swap(true, Ordering::AcqRel) {
            let _ = self.proxy.send_event(AppEvent::WakeUp);
        }
    }

    /// Re-arm before rendering so output arriving during the draw schedules another frame.
    pub fn rearm(&self) {
        self.queued.store(false, Ordering::Release);
    }

    pub fn is_queued(&self) -> bool {
        self.queued.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{MenuCommand, PlatformEvent};

    fn app_event(event: PlatformEvent) -> AppEvent {
        event.into()
    }

    #[test]
    fn platform_events_keep_their_meaning() {
        assert!(matches!(app_event(PlatformEvent::Reopen), AppEvent::Reopen));
        assert!(matches!(
            app_event(PlatformEvent::SystemAppearance { dark: true }),
            AppEvent::SystemAppearance { dark: true }
        ));
        assert!(matches!(
            app_event(PlatformEvent::SystemAppearance { dark: false }),
            AppEvent::SystemAppearance { dark: false }
        ));
        assert!(matches!(
            app_event(PlatformEvent::ToggleQuickTerminal),
            AppEvent::Menu(keymap::Action::ToggleQuickTerminal)
        ));
    }

    #[test]
    fn every_menu_command_maps_to_its_own_action() {
        use keymap::Action as A;
        let mut seen = Vec::new();
        for command in MenuCommand::ALL {
            let AppEvent::Menu(action) = app_event(PlatformEvent::Menu(command)) else {
                panic!("{command:?} is not a menu event");
            };
            assert!(!seen.contains(&action), "{command:?} repeats {action:?}");
            seen.push(action);
        }
        assert_eq!(seen[0], A::Quit);
        assert_eq!(seen[1], A::ToggleSecureInput);
        assert_eq!(*seen.last().unwrap(), A::CheckUpdate);
    }
}
