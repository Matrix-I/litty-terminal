//! Per-window state.

use std::sync::Arc;

use crate::{ai, config, mouse, renderer, scroll};
use crate::platform;
use crate::{HoveredPaneLink, Tab};

use winit::keyboard::{ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, Window};

use crate::terminal::history::HistorySearch;
use crate::renderer::Renderer;
use crate::terminal::search::SearchBar;
use crate::split::PaneRect;

pub(super) struct State {
    pub(super) ai: std::collections::BTreeMap<u64, ai::PaneState>,
    pub(super) window: Arc<Window>,
    pub(super) renderer: Renderer,
    pub(super) tabs: Vec<Tab>,
    pub(super) tab_titles: Vec<String>,
    pub(super) pane_rects: Vec<PaneRect>,
    pub(super) render_panes: Vec<renderer::RenderPane>,
    pub(super) active: usize,
    pub(super) mods: ModifiersState,
    pub(super) font_pt: f32,
    pub(super) pointer: PointerState,
    /// Last desktop notification shown, to rate-limit OSC 9 / 777 floods.
    pub(super) last_notification: Option<std::time::Instant>,
    /// Secure Keyboard Entry requested (macOS); held only while focused.
    pub(super) secure_input: bool,
    pub(super) config: config::Config,
    pub(super) config_mtime: Option<std::time::SystemTime>,
    /// Keys whose press Litty handled itself (shortcuts, search box), so their
    /// release is not reported to a Kitty-protocol application either.
    pub(super) consumed_keys: Vec<PhysicalKey>,
    /// Caps / Num Lock inferred from key presses (`keymap::infer_locks`), for
    /// platforms that do not report them.
    pub(super) locks: u8,
    pub(super) cursor_blink_epoch: std::time::Instant,
    pub(super) focused: bool,
    /// A global X11 key grab briefly loses focus. Defer autohide until that
    /// transient focus round-trip or the corresponding toggle has completed.
    pub(super) quick_autohide_at: Option<std::time::Instant>,
    pub(super) history_search: HistorySearch,
    /// Find-in-terminal box (`Cmd+F` / `Ctrl+Shift+F`).
    pub(super) search: SearchBar,
    pub(super) first_frame: bool,
    pub(super) ime: ImeState,
    /// A frame was skipped because an application is in synchronized output.
    pub(super) sync_pending: bool,
    /// When tab titles were last recomputed (they also refresh on OSC 0/2/7).
    pub(super) titles_refreshed_at: std::time::Instant,
    pub(super) requests: WindowRequests,
    /// The window is completely covered, minimized or on an inactive Space.
    pub(super) occluded: bool,
    /// This is the Quick Terminal window.
    pub(super) quick: bool,
    pub(super) inspector: InspectorState,
}

/// Terminal inspector overlay.
pub(super) struct InspectorState {
    /// Terminal inspector overlay is shown.
    pub(super) shown: bool,
    /// Last key sent to the terminal and its bytes, for the inspector.
    pub(super) last_key: String,
}

impl State {
    pub(super) fn invalidate_pane_state(&mut self, pane_id: u64) {
        self.ai.remove(&pane_id);
        if self.search.pane_id == Some(pane_id) {
            self.search.close();
        }
        if self.pointer.hovered_link.as_ref().is_some_and(|h| h.pane_id == pane_id) {
            self.pointer.hovered_link = None;
        }
        if self.pointer.scrollbar_drag == Some(pane_id) {
            self.pointer.scrollbar_drag = None;
        }
        if self.pointer.active_click_pane_id == Some(pane_id) {
            self.pointer.active_click_pane_id = None;
        }
        if self.pointer.last_mouse_report.as_ref().is_some_and(|(id, ..)| *id == pane_id) {
            self.pointer.last_mouse_report = None;
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        platform::cleanup_window(&self.window);
        if self.secure_input && self.focused {
            platform::set_secure_input(false);
        }
    }
}

/// Input-method composition.
pub(super) struct ImeState {
    /// IME composition in progress (Telex, Pinyin, ...); keys belong to the IME while non-empty.
    pub(super) preedit: String,
    /// Last cursor cell reported to the IME for candidate window placement.
    pub(super) cursor_cell: Option<(usize, usize)>,
}

/// Requests a window raises during an event; `App` acts on them afterwards.
pub(super) struct WindowRequests {
    /// The last tab closed; `App` removes the window (or hides the last one).
    pub(super) close: bool,
    /// A shortcut asked for a new window; `App` opens it after the event.
    pub(super) new_window: bool,
    /// New tab as a native macOS window tab (`macos-native-tabs`).
    pub(super) native_tab: bool,
    /// A binding asked to show / hide the Quick Terminal.
    pub(super) quick_terminal: bool,
    /// `Cmd+Q` / `Ctrl+Shift+Q`: `App` confirms across all windows, then exits.
    pub(super) quit: bool,
    /// Config reload requested; `App` propagates it to all windows.
    pub(super) reload_config: bool,
}

/// Mouse pointer, click sequence and hover tracking.
pub(super) struct PointerState {
    pub(super) mouse_pos: (f64, f64),
    pub(super) selecting: bool,
    pub(super) drag_moved: bool,
    pub(super) last_click_time: Option<std::time::Instant>,
    pub(super) last_click_cell: Option<(usize, usize)>,
    pub(super) active_click_pane_id: Option<u64>,
    pub(super) click_count: u8,
    /// The press that started this click sequence landed on an already selected
    /// block, so the double-click widens the selection to the line.
    pub(super) click_on_selected_block: bool,
    /// The current drag started with Shift+Click and moves only the selection end.
    pub(super) extend_drag: bool,
    /// Pane whose scrollbar thumb is being dragged.
    pub(super) scrollbar_drag: Option<u64>,
    pub(super) wheel_scroll: scroll::Accumulator,
    /// Mouse pointer outside links: the active pane's OSC 22 shape.
    pub(super) pointer_icon: CursorIcon,
    pub(super) hovered_link: Option<HoveredPaneLink>,
    pub(super) mouse_hidden: bool,
    /// Mouse button held while the application tracks the mouse (for drag reports).
    pub(super) mouse_button_down: Option<mouse::Button>,
    /// Last `(pane, row, col)` reported to a mouse-tracking application.
    pub(super) last_mouse_report: Option<(u64, usize, usize)>,
}
