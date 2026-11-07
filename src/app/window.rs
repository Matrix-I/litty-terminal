//! Window creation, sizing, config reload and per-frame rendering.

use super::inspector::inspector_lines;
use super::search_ui::refresh_search;
use super::state::{ImeState, InspectorState, PointerState, State, WindowRequests};
use super::tab_tearoff::TornTab;
use super::terminal_io::write_pty;

use std::sync::Arc;

use crate::terminal::grid;
use crate::render::atlas;
use crate::{config, scroll};
use crate::platform;
use crate::{next_id, AppEvent, Pane, Tab, Waker};

use parking_lot::Mutex;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::{CursorIcon, Window};

use crate::terminal::grid::Grid;
use super::history_ui::HistorySearch;
use crate::terminal::pty::Pty;
use crate::render::renderer::{Renderer, TabBarView};
use crate::terminal::search::SearchBar;

/// Drop the Quick Terminal from the top of its screen: full width, 40% height,
/// then bring it to the front.
pub(super) fn place_quick_terminal(s: &State) {
    if let Some(m) = s.window.current_monitor() {
        let (pos, size) = (m.position(), m.size());
        s.window.set_outer_position(pos);
        let _ = s.window.request_inner_size(PhysicalSize::new(size.width, size.height * 2 / 5));
    }
    s.window.set_visible(true);
    s.window.focus_window();
    platform::activate_app();
}

/// `font-feature` values as shaper features (validated when the config was parsed).
pub(super) fn font_features(cfg: &config::Config) -> Vec<atlas::FontFeature> {
    cfg.font_features.iter().filter_map(|f| atlas::parse_font_feature(f).ok()).collect()
}

/// Create a window with one tab whose shell starts in `cwd`.
pub(super) fn create_window(event_loop: &ActiveEventLoop, waker: &Waker, cwd: Option<&str>, quick: bool) -> std::io::Result<State> {
    build_window(event_loop, waker, cwd, quick, &mut None)
}

/// Open a window around a tab torn off another one. `torn` is emptied only once
/// the window exists, so on failure the caller can still put the tab back.
pub(super) fn create_window_for_tab(event_loop: &ActiveEventLoop, waker: &Waker, torn: &mut Option<TornTab>) -> std::io::Result<State> {
    build_window(event_loop, waker, None, false, torn)
}

fn build_window(
    event_loop: &ActiveEventLoop,
    waker: &Waker,
    cwd: Option<&str>,
    quick: bool,
    adopt: &mut Option<TornTab>,
) -> std::io::Result<State> {
    let cfg = config::Config::load();
    grid::set_default_colors(cfg.foreground, cfg.background);
    grid::set_palette(&cfg.palette);
    grid::set_scrollback_limit(cfg.scrollback_limit);
    grid::set_grapheme_default(cfg.grapheme_unicode);

    atlas::set_font_tweaks(cfg.adjust_cell_width, cfg.adjust_cell_height, cfg.font_thicken);
    atlas::set_font_features(&font_features(&cfg));
    let font = atlas::load_font(&cfg.font_family);
    // A torn-off tab keeps the font size (zoom) of the window it came from.
    let font_pt = adopt.as_ref().map_or_else(|| cfg.font_size.clamp(6.0, 72.0), |torn| torn.font_pt);

    let initial_inner_size = if let (Some(w_cols), Some(w_rows)) = (cfg.window_width, cfg.window_height) {
        let (cw_pt, ch_pt) = atlas::Atlas::cell_size_pt(&font, font_pt);
        let pad_x = cfg.window_padding_x as f64;
        let pad_y = cfg.window_padding_y as f64;
        LogicalSize::new(
            w_cols as f64 * cw_pt + 2.0 * pad_x,
            w_rows as f64 * ch_pt + 2.0 * pad_y,
        )
    } else {
        LogicalSize::new(960.0, 600.0)
    };

    // Linux draws its own title bar in the terminal's colors (the native frame
    // cannot be recolored); macOS makes its native title bar transparent instead.
    let csd = cfg!(target_os = "linux") && !quick;
    let attrs = Window::default_attributes()
        .with_title("Litty")
        // Build Quick Terminal in its final mode while hidden. Changing X11
        // decorations on an already mapped window causes spurious focus loss.
        .with_decorations(!quick && !csd)
        .with_window_level(if quick { winit::window::WindowLevel::AlwaysOnTop } else { winit::window::WindowLevel::Normal })
        .with_visible(!quick)
        .with_transparent(cfg.background_opacity < 1.0 || csd);
    // A torn-off tab keeps the size of its old window, so its grids do not reflow.
    let attrs = match adopt.as_ref() {
        Some(torn) => {
            let attrs = attrs.with_inner_size(torn.size);
            match torn.position {
                Some(position) => attrs.with_position(position),
                None => attrs,
            }
        }
        None => attrs.with_inner_size(initial_inner_size),
    };
    // Wayland app_id / X11 WM_CLASS must match `litty.desktop` and the
    // hicolor icon name, or the shell shows a generic icon.
    #[cfg(target_os = "linux")]
    let attrs = {
        use winit::platform::wayland::WindowAttributesExtWayland;
        attrs.with_name("litty", "litty")
    };
    // Native tabs (`macos-native-tabs`) group under one identifier. Created
    // hidden so AppKit cannot merge a New Window into a tab of the focused one
    // before tabbing is turned off for it; native tabs re-enable it.
    #[cfg(target_os = "macos")]
    let attrs = {
        use winit::platform::macos::WindowAttributesExtMacOS;
        attrs.with_tabbing_identifier("litty").with_visible(false)
    };
    let window = Arc::new(event_loop.create_window(attrs).map_err(std::io::Error::other)?);
    platform::init_clipboard(&window);
    platform::set_tabbing_allowed(&window, false);
    if !quick { window.set_visible(true); }
    // Composed input (Vietnamese Telex/VNI, CJK) arrives as `WindowEvent::Ime`.
    window.set_ime_allowed(true);
    let dark = window.theme() != Some(winit::window::Theme::Light);
    grid::set_color_scheme_dark(dark);
    // `theme = light:X,dark:Y` was parsed before the appearance was known.
    let cfg = if !dark && cfg.theme.as_deref().is_some_and(config::theme_is_conditional) {
        let cfg = config::Config::load();
        grid::set_default_colors(cfg.foreground, cfg.background);
        grid::set_palette(&cfg.palette);
        cfg
    } else {
        cfg
    };

    platform::apply_window_config(&window, &(&cfg).into());
    platform::set_dock_icon();

    let font_px = (font_pt * window.scale_factor() as f32).round();

    let mut renderer = Renderer::new(
        window.clone(),
        font,
        font_px,
        cfg.background_opacity,
        cfg.cursor_style.shader_id(),
    );
    renderer.set_titlebar(csd);
    renderer.set_padding(cfg.window_padding_x, cfg.window_padding_y, cfg.window_padding_balance);
    renderer.set_colors(cfg.cursor_color, cfg.selection_background, cfg.selection_foreground);
    renderer.set_contrast(cfg.unfocused_split_opacity, cfg.unfocused_split_fill, cfg.minimum_contrast);
    if !cfg.custom_shaders.is_empty() {
        renderer.set_custom_shaders(&cfg.custom_shaders, cfg.custom_shader_animation);
    }

    let sz = window.inner_size();
    renderer.resize(sz.width, sz.height);

    let adopted = adopt.take();
    let is_adopted = adopted.is_some();
    let (mut tab, ai) = match adopted {
        Some(torn) => (torn.tab, torn.ai.into_iter().collect()),
        None => {
            let (cols, rows) = renderer.grid_size_for_tabs(usize::from(csd));
            let (cw, ch) = renderer.cell_size();
            let pane = spawn_pane(
                next_id(),
                cols,
                rows,
                (cw, ch),
                cwd,
                Some(cfg.shell_setup()),
                waker,
            )?;
            (Tab::new(next_id(), pane), std::collections::BTreeMap::new())
        }
    };
    let title = tab_title(&mut tab);
    let bar_title = if csd { let (user, host) = user_host(); title_with_host(&title, user, host) } else { String::new() };

    let config_mtime = config::config_mtime();
    let mut state = State {
        ai,
        window,
        renderer,
        tabs: vec![tab],
        tab_titles: vec![title],
        tab_anim: Default::default(),
        pane_rects: Vec::new(),
        render_panes: Vec::new(),
        active: 0,
        mods: crate::keymap::Mods::default(),
        font_pt,
        pointer: PointerState {
            mouse_pos: (0.0, 0.0),
            selecting: false,
            drag_moved: false,
            last_click_time: None,
            last_title_click: None,
            on_edge: false,
            last_click_cell: None,
            active_click_pane_id: None,
            click_count: 0,
            click_on_selected_block: false,
            extend_drag: false,
            scrollbar_drag: None,
            tab_drag: None,
            drag_holder: None,
            wheel_scroll: scroll::Accumulator::default(),
            pointer_icon: CursorIcon::Default,
            hovered_link: None,
            mouse_hidden: false,
            mouse_button_down: None,
            last_mouse_report: None,
        },
        last_notification: None,
        secure_input: false,
        config: cfg,
        config_mtime,
        consumed_keys: Vec::new(),
        locks: 0,
        cursor_blink_epoch: std::time::Instant::now(),
        focused: !quick,
        quick_autohide_at: None,
        history_search: HistorySearch::new(),
        search: SearchBar::new(),
        first_frame: true,
        ime: ImeState { preedit: String::new(), cursor_cell: None },
        sync_pending: false,
        titles_refreshed_at: std::time::Instant::now(),
        titled_tab: 0,
        requests: WindowRequests {
            close: false,
            new_window: false,
            native_tab: false,
            quick_terminal: false,
            quit: false,
            reload_config: false,
            tear_off: None,
            move_torn: None,
            drag_ping: None,
            drag_release: None,
        },
        occluded: false,
        quick,
        csd,
        bar_title,
        inspector: InspectorState { shown: false, last_key: String::new() },
    };
    super::csd::update_corner_radius(&mut state);
    if is_adopted {
        // The adopted panes were sized for the old window.
        sync_size(&mut state);
    }
    Ok(state)
}

pub(super) fn spawn_pane(
    pane_id: u64,
    cols: usize,
    rows: usize,
    (cw, ch): (u32, u32),
    cwd: Option<&str>,
    shell: Option<crate::terminal::shell_integration::ShellSetup>,
    waker: &Waker,
) -> std::io::Result<Pane> {
    let grid = Arc::new(Mutex::new(Grid::new(cols, rows)));
    let pty = Pty::spawn_in(cols as u16, rows as u16, cw, ch, cwd, shell)?;
    let on_output = waker.clone();
    let proxy = waker.proxy.clone();
    pty.spawn_reader(grid.clone(), move || on_output.wake(), move || {
        let _ = proxy.send_event(AppEvent::ChildExit(pane_id));
    })?;
    Ok(Pane {
        id: pane_id,
        grid,
        pty,
        cached_cwd: None,
        last_cwd_check: std::time::Instant::now() - std::time::Duration::from_secs(10),
        cached_running: false,
        last_running_check: std::time::Instant::now() - std::time::Duration::from_secs(10),
    })
}

pub(super) fn tab_title(tab: &mut Tab) -> String {
    let mut title = if let Some(active) = tab.active_pane_mut() {
        let grid_title = {
            let g = active.grid.lock();
            if !g.title.is_empty() {
                Some(g.title.clone())
            } else {
                None
            }
        };
        if let Some(t) = grid_title {
            t
        } else if let Some(cwd) = active.cwd() {
            static HOME_USER: std::sync::OnceLock<(Option<String>, String)> = std::sync::OnceLock::new();
            let (home, user) = HOME_USER.get_or_init(|| {
                let h = std::env::var("HOME").ok();
                let u = std::env::var("USER").unwrap_or_default();
                (h, u)
            });
            let short_dir = if let Some(home) = home {
                if &cwd == home {
                    "~".to_string()
                } else if let Some(rest) = cwd.strip_prefix(home) {
                    format!("~{rest}")
                } else {
                    cwd
                }
            } else {
                cwd
            };
            if user.is_empty() {
                short_dir
            } else {
                format!("{user}:{short_dir}")
            }
        } else {
            "shell".into()
        }
    } else {
        "shell".into()
    };

    if tab.zoomed_pane_id.is_some() {
        title.push_str(" [Z]");
    }
    // OSC 9;4 progress and an unseen bell show in front of the title.
    let progress = tab.active_pane().and_then(|p| p.grid.lock().progress);
    let prefix = match progress {
        Some((2, pct)) => format!("✖ {pct}% "),
        Some((3, _)) => "⋯ ".to_string(),
        Some((4, pct)) => format!("⏸ {pct}% "),
        Some((_, pct)) => format!("{pct}% "),
        None => String::new(),
    };
    let bell = if tab.bell { "🔔 " } else { "" };
    format!("{bell}{prefix}{title}")
}

/// `user@host` of this machine, for the title bar.
fn user_host() -> &'static (String, String) {
    static USER_HOST: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
    USER_HOST.get_or_init(|| {
        let user = std::env::var("USER").unwrap_or_default();
        let mut buf = [0u8; 256];
        // SAFETY: `buf` is valid for `buf.len()` bytes; gethostname NUL-terminates within it or fails.
        let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
        let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let host = if ok { String::from_utf8_lossy(&buf[..len]).into_owned() } else { String::new() };
        (user, host)
    })
}

/// Title bar text: a bare directory (`~/dir`, `/dir`) or `user:dir` from a shell that
/// sends no host becomes `user@host:dir`, so every tab shows the same form. Titles that
/// already carry a host, and program titles (`vim`), are kept.
fn title_with_host(title: &str, user: &str, host: &str) -> String {
    let path = if title.starts_with('~') || title.starts_with('/') {
        title
    } else if let Some(rest) = title.strip_prefix(user).and_then(|r| r.strip_prefix(':')).filter(|_| !user.is_empty()) {
        rest
    } else {
        return title.to_string();
    };
    if user.is_empty() || host.is_empty() {
        return title.to_string();
    }
    format!("{user}@{host}:{path}")
}

/// The window title must be rewritten when a tab title changed or another tab became
/// active: switching tabs leaves every tab's own title untouched.
fn window_title_stale(titles_changed: bool, shown_tab: usize, active: usize) -> bool {
    titles_changed || shown_tab != active
}

pub(super) fn refresh_tab_titles(s: &mut State) {
    s.titles_refreshed_at = std::time::Instant::now();
    let mut changed = s.tab_titles.len() != s.tabs.len();
    s.tab_titles.resize_with(s.tabs.len(), String::new);
    for (dst, tab) in s.tab_titles.iter_mut().zip(&mut s.tabs) {
        let new_title = tab_title(tab);
        if *dst != new_title {
            *dst = new_title;
            changed = true;
        }
    }
    if s.csd {
        let (user, host) = user_host();
        let bar = s.tab_titles.get(s.active).map_or_else(String::new, |t| title_with_host(t, user, host));
        if bar != s.bar_title {
            s.bar_title = bar;
            changed = true;
        }
    }
    if window_title_stale(changed, s.titled_tab, s.active) {
        s.titled_tab = s.active;
        if s.show_bar() {
            s.renderer.invalidate();
        }
        if let Some(active_title) = s.tab_titles.get(s.active) {
            let title = s.config.title.as_deref().unwrap_or(active_title);
            let lock = if s.secure_input { "🔒 " } else { "" };
            s.window.set_title(&format!("{lock}{title}"));
            #[cfg(target_os = "macos")]
            if let Some(tab) = s.tabs.get_mut(s.active)
                && let Some(cwd) = tab.active_pane_mut().and_then(|p| p.cwd()) {
                    platform::set_window_represented_filename(&s.window, &cwd);
                }
        }
    }
}

pub(super) fn cursor_blinks(s: &State) -> bool {
    match s.config.cursor_style_blink {
        Some(b) => b,
        None => {
            if let Some(tab) = s.tabs.get(s.active) {
                if let Some(pane) = tab.active_pane() {
                    let g = pane.grid.lock();
                    // DECSCUSR odd values (and 0 = default) are the blinking styles.
                    matches!(g.modes.cursor_shape, 0 | 1 | 3 | 5)
                } else {
                    true
                }
            } else {
                true
            }
        }
    }
}

pub(super) fn cursor_is_visible_now(s: &State) -> bool {
    if !s.focused {
        return true;
    }
    if !cursor_blinks(s) {
        return true;
    }
    let elapsed = s.cursor_blink_epoch.elapsed().as_millis();
    (elapsed % 1000) < 500
}

/// Grab (or, with `None`, release) the Quick Terminal global hotkey. A key the
/// platform has no code for releases it and says so.
pub(super) fn set_quick_terminal_hotkey(trigger: Option<&crate::keymap::Trigger>) {
    let hotkey = trigger.and_then(|t| {
        let hotkey = crate::keymap::global_hotkey(t);
        if hotkey.is_none() {
            eprintln!("litty: Quick Terminal hotkey: key not supported for a global hotkey");
        }
        hotkey
    });
    platform::set_global_hotkey(hotkey);
}

pub(super) fn reload_config(s: &mut State) {
    let old = s.config.clone();
    let new = config::Config::load();
    s.config_mtime = config::config_mtime();
    if new == old {
        return;
    }

    grid::set_default_colors(new.foreground, new.background);
    grid::set_palette(&new.palette);

    platform::apply_window_config(&s.window, &(&new).into());

    s.renderer.set_bg_opacity(new.background_opacity);
    s.renderer.set_cursor_style(new.cursor_style.shader_id());
    s.renderer.set_colors(new.cursor_color, new.selection_background, new.selection_foreground);
    s.renderer.set_contrast(new.unfocused_split_opacity, new.unfocused_split_fill, new.minimum_contrast);
    if (&new.custom_shaders, new.custom_shader_animation) != (&old.custom_shaders, old.custom_shader_animation) {
        s.renderer.set_custom_shaders(&new.custom_shaders, new.custom_shader_animation);
    }
    // Applies to terminals created or resized from now on.
    grid::set_scrollback_limit(new.scrollback_limit);
    grid::set_grapheme_default(new.grapheme_unicode);
    let padding = (new.window_padding_x, new.window_padding_y, new.window_padding_balance);
    if padding != (old.window_padding_x, old.window_padding_y, old.window_padding_balance) {
        s.renderer.set_padding(padding.0, padding.1, padding.2);
        sync_size(s);
    }

    let font_changed = new.font_family != old.font_family;
    let size_changed = (new.font_size - old.font_size).abs() > 0.001;
    // Compared per window: another window may already have set the new tweaks.
    atlas::set_font_tweaks(new.adjust_cell_width, new.adjust_cell_height, new.font_thicken);
    atlas::set_font_features(&font_features(&new));
    if new.quick_terminal_hotkey != old.quick_terminal_hotkey {
        set_quick_terminal_hotkey(new.quick_terminal_hotkey.as_ref());
    }
    let tweaks_changed = (new.adjust_cell_width, new.adjust_cell_height, new.font_thicken, &new.font_features)
        != (old.adjust_cell_width, old.adjust_cell_height, old.font_thicken, &old.font_features);
    if font_changed || size_changed || tweaks_changed {
        s.font_pt = new.font_size;
        let font = atlas::load_font(&new.font_family);
        let sf = s.window.scale_factor();
        let font_px = (s.font_pt * sf as f32).round();
        s.renderer.set_fonts(font, font_px);
        sync_size(s);
    }

    if !new.mouse_hide_while_typing && s.pointer.mouse_hidden {
        s.window.set_cursor_visible(true);
        s.pointer.mouse_hidden = false;
    }

    s.config = new;
    s.renderer.invalidate();
    s.window.request_redraw();
}

pub(super) fn render_active(s: &mut State) {
    if s.tabs.is_empty() {
        return;
    }
    if s.active >= s.tabs.len() {
        s.active = s.tabs.len() - 1;
    }
    if s.search.is_open {
        // Follow focus to another pane or tab; mirror the grid's match status.
        if s.search.pane_id != Some(s.tabs[s.active].active_pane_id) {
            refresh_search(s);
        }
        if let Some(pane) = s.tabs[s.active].active_pane() {
            (s.search.current, s.search.total) = pane.grid.lock().search_status();
        }
    }
    // Synchronized output (DECSET 2026): wait for the application to finish its
    // frame, but never longer than 150ms.
    let syncing = s.tabs[s.active].panes.iter().any(|p| {
        let g = p.grid.lock();
        g.modes.sync_output && g.modes.sync_started.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(150))
    }) || s.tabs[s.active].panes.iter().any(|p| s.ai.get(&p.id).is_some_and(|ai| ai.holds_render()));
    s.sync_pending = syncing;
    if syncing {
        return;
    }

    let mut titles_changed = false;
    for pane in s.tabs.iter().flat_map(|t| &t.panes) {
        titles_changed |= std::mem::take(&mut pane.grid.lock().title_changed);
    }
    if titles_changed || s.titles_refreshed_at.elapsed() >= std::time::Duration::from_secs(1) {
        refresh_tab_titles(s);
    }
    let inspector = s.inspector.shown.then(|| inspector_lines(s));
    let animating = s.tab_anim.tick(std::time::Instant::now(), s.tabs.len());
    let tab_anim_dirty = s.tab_anim.take_dirty();
    let tab_bar = s.show_bar().then(|| TabBarView {
        shifts: s.tab_anim.shifts(),
        dragged: s.tab_anim.dragged(),
        dirty: tab_anim_dirty,
        titlebar: s.csd,
        title: s.config.title.as_deref().unwrap_or(&s.bar_title),
        ..TabBarView::new(&s.tab_titles, s.active)
    });
    let blink_on = cursor_is_visible_now(s);

    let bands = s.bands();
    let (total_cols, total_rows) = s.renderer.grid_size_for_tabs(bands);

    let tab = &s.tabs[s.active];
    tab.prepare_render_panes(total_cols, total_rows, &mut s.pane_rects, &mut s.render_panes);
    let render_panes = &s.render_panes;

    let hovered_link = s.pointer.hovered_link.as_ref().map(|l| (l.pane_id, &l.link));

    s.renderer.render_panes(
        render_panes,
        tab_bar,
        hovered_link,
        blink_on,
        Some(s.history_search.view()),
        &s.search,
        &s.ime.preedit,
        inspector.as_deref(),
    );
    // Tabs are still sliding: ask for the next frame (vsync paces it); idle stops here.
    if animating {
        s.window.request_redraw();
    }

    // Keep the IME candidate window next to the text cursor.
    let cursor_cell = render_panes.iter().find(|p| p.is_active).and_then(|p| {
        let (cx, cy) = p.grid.lock().cursor_pos()?;
        Some((p.rect.row + cy, p.rect.col + cx))
    });
    if cursor_cell.is_some() && cursor_cell != s.ime.cursor_cell {
        s.ime.cursor_cell = cursor_cell;
        if let Some((row, col)) = cursor_cell {
            let (x, y) = s.renderer.cell_origin_px(row, col, bands);
            let (cw, ch) = s.renderer.cell_size();
            s.window.set_ime_cursor_area(PhysicalPosition::new(x, y), PhysicalSize::new(cw, ch));
        }
    }

    s.render_panes.clear();
    if s.first_frame {
        s.first_frame = false;
        platform::release_free_memory();
    }
}

pub(super) fn sync_size(s: &mut State) {
    let bands = s.bands();
    let (total_cols, total_rows) = s.renderer.grid_size_for_tabs(bands);
    let (cw, ch) = s.renderer.cell_size();
    for tab in &mut s.tabs {
        let rects = tab.compute_pane_rects(total_cols, total_rows);
        for rect in rects {
            if let Some(p) = tab.panes.iter().find(|p| p.id == rect.id) {
                let changed = {
                    let mut g = p.grid.lock();
                    g.cell_px = (cw as u16, ch as u16);
                    if (g.cols, g.rows) == (rect.cols, rect.rows) {
                        false
                    } else {
                        g.resize(rect.cols, rect.rows);
                        true
                    }
                };
                if changed {
                    p.pty.resize(rect.cols as u16, rect.rows as u16, cw, ch);
                }
            }
        }
    }
    // Reflow cleared the matches.
    if s.search.is_open {
        refresh_search(s);
    }
}

pub(super) fn zoom(s: &mut State, delta_pt: f32) {
    s.font_pt = if delta_pt == 0.0 {
        s.config.font_size
    } else {
        (s.font_pt + delta_pt).clamp(6.0, 72.0)
    };
    let sf = s.window.scale_factor();
    s.renderer.set_font_px((s.font_pt * sf as f32).round(), sf);
    sync_size(s);
    s.window.request_redraw();
}

/// Follow a Light/Dark switch: re-resolve `theme = light:X,dark:Y` and tell
/// applications that set DECSET 2031.
pub(super) fn apply_system_appearance(s: &mut State) {
    if s.config.theme.as_deref().is_some_and(config::theme_is_conditional) {
        s.requests.reload_config = true;
    }
    for pane in s.tabs.iter().flat_map(|t| &t.panes) {
        let report_scheme = pane.grid.lock().modes.color_scheme_reports;
        if report_scheme {
            write_pty(pane, grid::color_scheme_report());
        }
    }
}

#[cfg(test)]
mod title_tests {
    use super::{title_with_host, window_title_stale};

    #[test]
    fn switching_tabs_rewrites_the_window_title() {
        assert!(window_title_stale(false, 0, 1));
        assert!(window_title_stale(true, 1, 1));
        assert!(!window_title_stale(false, 1, 1));
    }

    #[test]
    fn bare_directories_gain_user_and_host() {
        assert_eq!(title_with_host("~/Downloads", "me", "box"), "me@box:~/Downloads");
        assert_eq!(title_with_host("/etc", "me", "box"), "me@box:/etc");
        assert_eq!(title_with_host("me:~/src", "me", "box"), "me@box:~/src");
    }

    #[test]
    fn titles_with_a_host_or_a_program_are_kept() {
        assert_eq!(title_with_host("me@box:~/src", "me", "box"), "me@box:~/src");
        assert_eq!(title_with_host("root@srv:/var", "me", "box"), "root@srv:/var");
        assert_eq!(title_with_host("vim notes.md", "me", "box"), "vim notes.md");
        assert_eq!(title_with_host("shell", "me", "box"), "shell");
    }

    #[test]
    fn unknown_user_or_host_leaves_the_title_alone() {
        assert_eq!(title_with_host("~/x", "", "box"), "~/x");
        assert_eq!(title_with_host("~/x", "me", ""), "~/x");
    }
}
