//! Window creation, sizing, config reload and per-frame rendering.

use super::inspector::inspector_lines;
use super::search_ui::refresh_search;
use super::state::{InspectorState, State};
use super::terminal_io::write_pty;

use std::sync::Arc;

use crate::{atlas, config, grid, hotkey, macos, scroll};
use crate::{next_id, AppEvent, Pane, Tab, Waker};

use parking_lot::Mutex;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, Window};

use crate::grid::Grid;
use crate::history::HistorySearch;
use crate::pty::Pty;
use crate::renderer::Renderer;
use crate::search::SearchBar;

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
    #[cfg(target_os = "macos")]
    macos::activate_app();
}

/// `font-feature` values as shaper features (validated when the config was parsed).
pub(super) fn font_features(cfg: &config::Config) -> Vec<rustybuzz::Feature> {
    cfg.font_features.iter().filter_map(|f| atlas::parse_font_feature(f).ok()).collect()
}

/// Create a window with one tab whose shell starts in `cwd`.
pub(super) fn create_window(event_loop: &ActiveEventLoop, waker: &Waker, cwd: Option<&str>, quick: bool) -> std::io::Result<State> {
    let cfg = config::Config::load();
    grid::set_default_colors(cfg.foreground, cfg.background);
    grid::set_palette(&cfg.palette);
    grid::set_scrollback_limit(cfg.scrollback_limit);
    grid::set_grapheme_default(cfg.grapheme_unicode);

    atlas::set_font_tweaks(cfg.adjust_cell_width, cfg.adjust_cell_height, cfg.font_thicken);
    atlas::set_font_features(&font_features(&cfg));
    let font = atlas::load_font(&cfg.font_family);
    let font_pt = cfg.font_size.clamp(6.0, 72.0);

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

    let attrs = Window::default_attributes()
        .with_title("Litty")
        // Build Quick Terminal in its final mode while hidden. Changing X11
        // decorations on an already mapped window causes spurious focus loss.
        .with_decorations(!quick)
        .with_window_level(if quick { winit::window::WindowLevel::AlwaysOnTop } else { winit::window::WindowLevel::Normal })
        .with_visible(!quick)
        .with_transparent(cfg.background_opacity < 1.0)
        .with_inner_size(initial_inner_size);
    // Native tabs (`macos-native-tabs`) group under one identifier. Created
    // hidden so AppKit cannot merge a New Window into a tab of the focused one
    // before tabbing is turned off for it; native tabs re-enable it.
    #[cfg(target_os = "macos")]
    let attrs = {
        use winit::platform::macos::WindowAttributesExtMacOS;
        attrs.with_tabbing_identifier("litty").with_visible(false)
    };
    let window = Arc::new(event_loop.create_window(attrs).map_err(std::io::Error::other)?);
    macos::set_tabbing_allowed(&window, false);
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

    #[cfg(target_os = "macos")]
    {
        macos::apply_macos_window_config(&window, &cfg);
        macos::set_dock_icon();
    }

    let font_px = (font_pt * window.scale_factor() as f32).round();

    let mut renderer = Renderer::new(
        window.clone(),
        font,
        font_px,
        cfg.background_opacity,
        cfg.cursor_style.shader_id(),
    );
    renderer.set_padding(cfg.window_padding_x, cfg.window_padding_y, cfg.window_padding_balance);
    renderer.set_colors(cfg.cursor_color, cfg.selection_background, cfg.selection_foreground);
    renderer.set_contrast(cfg.unfocused_split_opacity, cfg.unfocused_split_fill, cfg.minimum_contrast);
    if !cfg.custom_shaders.is_empty() {
        renderer.set_custom_shaders(&cfg.custom_shaders, cfg.custom_shader_animation);
    }

    let sz = window.inner_size();
    renderer.resize(sz.width, sz.height);

    let (cols, rows) = renderer.grid_size_for_tabs(false);
    let (cw, ch) = renderer.cell_size();

    let pane = spawn_pane(
        next_id(),
        cols,
        rows,
        (cw, ch),
        cwd,
        Some((cfg.shell_integration, cfg.shell_integration_features)),
        waker,
    )?;
    let mut tab = Tab::new(next_id(), pane);
    let title = tab_title(&mut tab);

    let config_mtime = config::config_mtime();
    Ok(State {
        ai: std::collections::BTreeMap::new(),
        window,
        renderer,
        tabs: vec![tab],
        tab_titles: vec![title],
        pane_rects: Vec::new(),
        render_panes: Vec::new(),
        active: 0,
        mods: ModifiersState::default(),
        font_pt,
        mouse_pos: (0.0, 0.0),
        selecting: false,
        drag_moved: false,
        last_click_time: None,
        last_click_cell: None,
        active_click_pane_id: None,
        click_count: 0,
        click_on_selected_block: false,
        extend_drag: false,
        scrollbar_drag: None,
        wheel_scroll: scroll::Accumulator::default(),
        pointer_icon: CursorIcon::Default,
        last_notification: None,
        secure_input: false,
        hovered_link: None,
        config: cfg,
        config_mtime,
        mouse_hidden: false,
        consumed_keys: Vec::new(),
        locks: 0,
        cursor_blink_epoch: std::time::Instant::now(),
        focused: !quick,
        quick_autohide_at: None,
        history_search: HistorySearch::new(),
        search: SearchBar::new(),
        first_frame: true,
        ime_preedit: String::new(),
        ime_cursor_cell: None,
        mouse_button_down: None,
        last_mouse_report: None,
        sync_pending: false,
        titles_refreshed_at: std::time::Instant::now(),
        close_requested: false,
        occluded: false,
        new_window_requested: false,
        native_tab_requested: false,
        quick,
        quick_terminal_requested: false,
        quit_requested: false,
        reload_config_requested: false,
        inspector: InspectorState { shown: false, last_key: String::new() },
    })
}

pub(super) fn spawn_pane(
    pane_id: u64,
    cols: usize,
    rows: usize,
    (cw, ch): (u32, u32),
    cwd: Option<&str>,
    shell: Option<(crate::config::ShellIntegration, crate::config::ShellFeatures)>,
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
    if changed {
        if s.tabs.len() > 1 {
            s.renderer.invalidate();
        }
        if let Some(active_title) = s.tab_titles.get(s.active) {
            let title = s.config.title.as_deref().unwrap_or(active_title);
            let lock = if s.secure_input { "🔒 " } else { "" };
            s.window.set_title(&format!("{lock}{title}"));
            #[cfg(target_os = "macos")]
            if let Some(tab) = s.tabs.get_mut(s.active)
                && let Some(cwd) = tab.active_pane_mut().and_then(|p| p.cwd()) {
                    macos::set_window_represented_filename(&s.window, &cwd);
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
                    matches!(g.cursor_shape, 0 | 1 | 3 | 5)
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

pub(super) fn reload_config(s: &mut State) {
    let old = s.config.clone();
    let new = config::Config::load();
    s.config_mtime = config::config_mtime();
    if new == old {
        return;
    }

    grid::set_default_colors(new.foreground, new.background);
    grid::set_palette(&new.palette);

    #[cfg(target_os = "macos")]
    macos::apply_macos_window_config(&s.window, &new);

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
        macos::set_global_hotkey(new.quick_terminal_hotkey.as_ref());
        hotkey::set_global_hotkey(new.quick_terminal_hotkey.as_ref());
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

    if !new.mouse_hide_while_typing && s.mouse_hidden {
        s.window.set_cursor_visible(true);
        s.mouse_hidden = false;
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
        g.sync_output && g.sync_started.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(150))
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
    let tab_bar = (s.tabs.len() > 1).then_some((s.tab_titles.as_slice(), s.active));
    let blink_on = cursor_is_visible_now(s);

    let show_tabs = s.tabs.len() > 1;
    let (total_cols, total_rows) = s.renderer.grid_size_for_tabs(show_tabs);

    let tab = &s.tabs[s.active];
    tab.prepare_render_panes(total_cols, total_rows, &mut s.pane_rects, &mut s.render_panes);
    let render_panes = &s.render_panes;

    let hovered_link = s.hovered_link.as_ref().map(|l| (l.pane_id, &l.link));

    s.renderer.render_panes(
        render_panes,
        tab_bar,
        hovered_link,
        blink_on,
        Some(&s.history_search),
        &s.search,
        &s.ime_preedit,
        inspector.as_deref(),
    );

    // Keep the IME candidate window next to the text cursor.
    let cursor_cell = render_panes.iter().find(|p| p.is_active).and_then(|p| {
        let (cx, cy) = p.grid.lock().cursor_pos()?;
        Some((p.rect.row + cy, p.rect.col + cx))
    });
    if cursor_cell.is_some() && cursor_cell != s.ime_cursor_cell {
        s.ime_cursor_cell = cursor_cell;
        if let Some((row, col)) = cursor_cell {
            let (x, y) = s.renderer.cell_origin_px(row, col, show_tabs);
            let (cw, ch) = s.renderer.cell_size();
            s.window.set_ime_cursor_area(PhysicalPosition::new(x, y), PhysicalSize::new(cw, ch));
        }
    }

    s.render_panes.clear();
    if s.first_frame {
        s.first_frame = false;
        macos::release_free_memory();
    }
}

pub(super) fn sync_size(s: &mut State) {
    let show_tabs = s.tabs.len() > 1;
    let (total_cols, total_rows) = s.renderer.grid_size_for_tabs(show_tabs);
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
        s.reload_config_requested = true;
    }
    for pane in s.tabs.iter().flat_map(|t| &t.panes) {
        let report_scheme = pane.grid.lock().color_scheme_reports;
        if report_scheme {
            write_pty(pane, grid::color_scheme_report());
        }
    }
}
