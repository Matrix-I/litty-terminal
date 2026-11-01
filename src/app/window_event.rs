//! Window event routing: `handle_window_event` and its per-event handlers.

use super::pointer::alt_screen_scroll;
use super::pointer::drag_scrollbar;
use super::pointer::pane_cell_at_mouse;
use super::pointer::report_mouse;
use super::pointer::scrollbar_at_mouse;
use super::pointer::update_hovered_link;
use super::tabs::close_last_window;
use super::tabs::close_tab_at;
use super::tabs::confirm_close;
use super::tabs::duplicate_active_tab;
use super::tabs::select_tab;
use super::terminal_io::write_pane_input;
use super::terminal_io::write_pty;
use super::window::apply_system_appearance;
use super::window::refresh_tab_titles;
use super::window::render_active;
use super::window::sync_size;
use super::App;
use super::ime::handle_ime;
use super::keyboard::handle_key;

use crate::{config, grid, macos, mouse, scroll};

use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::window::WindowId;

use crate::grid::Selection;
use crate::paste::{history_insert_payload, paste_payload, shell_quote};
use crate::platform::clipboard::copy_to_clipboard;
use crate::platform::process::open_target;
use crate::renderer::TabBarAction;


impl App {
    pub(super) fn handle_window_event(&mut self, id: WindowId, event: WindowEvent) {
        let event = match event {
            WindowEvent::CursorMoved { position, .. } => return self.on_cursor_moved(id, position),
            WindowEvent::MouseInput { state: btn_state, button, .. } => return self.on_mouse_input(id, btn_state, button),
            WindowEvent::MouseWheel { delta, phase, .. } => return self.on_mouse_wheel(id, delta, phase),
            WindowEvent::Focused(focused) => return self.on_focused(id, focused),
            WindowEvent::RedrawRequested => return self.on_redraw_requested(id),
            other => other,
        };
        let waker = self.waker.clone();
        let Some(s) = self.window_mut(id) else { return };
        match event {
            WindowEvent::CloseRequested => {
                if confirm_close(&s.config, s.tabs.iter().flat_map(|t| &t.panes), "this window") {
                    s.ai.clear();
                    s.tabs.clear();
                    close_last_window(s);
                }
            }

            WindowEvent::ModifiersChanged(m) => {
                s.mods = m.state();
                update_hovered_link(s);
            }

            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    s.renderer.resize(size.width, size.height);
                    sync_size(s);
                    s.window.request_redraw();
                }
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let px = (s.font_pt * scale_factor as f32).round();
                s.renderer.set_font_px(px, scale_factor);
                let sz = s.window.inner_size();
                s.renderer.resize(sz.width, sz.height);
                sync_size(s);
                #[cfg(target_os = "macos")]
                macos::apply_macos_window_config(&s.window, &s.config);
                s.window.request_redraw();
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed && s.renderer.set_bell_border(false) {
                    s.window.request_redraw();
                }
                if s.ime.preedit.is_empty() {
                    handle_key(s, &event, &waker);
                }
            }

            WindowEvent::Ime(ime) => handle_ime(s, ime),

            // Dropped files paste their shell-quoted paths, separated by spaces.
            WindowEvent::DroppedFile(path) => {
                if let Some(tab) = s.tabs.get(s.active)
                    && let Some(pane) = tab.active_pane() {
                        let bracketed = pane.grid.lock().modes.bracketed_paste;
                        let text = format!("{} ", shell_quote(&path.to_string_lossy()));
                        write_pane_input(pane,&mut s.ai,&paste_payload(&text, bracketed));
                    }
            }

            WindowEvent::ThemeChanged(theme) => {
                let dark = theme == winit::window::Theme::Dark;
                if dark != grid::color_scheme_is_dark() {
                    grid::set_color_scheme_dark(dark);
                    apply_system_appearance(s);
                }
            }

            WindowEvent::Occluded(occluded) => {
                s.renderer.capture_occlusion(occluded);
                s.occluded = occluded;
                if occluded {
                    s.renderer.shrink_drawables();
                    #[cfg(target_os = "macos")]
                    macos::release_free_memory();
                } else {
                    let sz = s.window.inner_size();
                    if sz.width > 0 && sz.height > 0 {
                        s.renderer.resize(sz.width, sz.height);
                    }
                    s.renderer.invalidate();
                    s.window.request_redraw();
                }
            }

            _ => {}
        }
    }

    pub(super) fn on_cursor_moved(&mut self, id: WindowId, position: PhysicalPosition<f64>) {
        let Some(s) = self.window_mut(id) else { return };
        s.pointer.mouse_pos = (position.x, position.y);
        if s.tabs.is_empty() {
            return;
        }
        if s.pointer.mouse_hidden {
            s.window.set_cursor_visible(true);
            s.pointer.mouse_hidden = false;
        }
        if s.inspector.shown {
            s.window.request_redraw();
        }
        s.renderer.set_mouse(position.x, position.y);
        if let Some(pane_id) = s.pointer.scrollbar_drag {
            drag_scrollbar(s, pane_id);
            return;
        }
        if !s.pointer.selecting && !s.history_search.is_open {
            let button = s.pointer.mouse_button_down.unwrap_or(mouse::Button::None);
            if report_mouse(s, button, mouse::Action::Motion) {
                return;
            }
        }
        if s.pointer.selecting {
            if let (Some(start), Some(pane_id)) = (s.pointer.last_click_cell, s.pointer.active_click_pane_id) {
                let show_tabs = s.tabs.len() > 1;
                let (cols, rows) = s.renderer.grid_size_for_tabs(show_tabs);
                if let Some((cell_row, cell_col)) = s.renderer.pixel_to_cell(position.x, position.y, show_tabs, cols, rows) {
                    let tab = &s.tabs[s.active];
                    let rects = tab.compute_pane_rects(cols, rows);
                    if let Some(target) = rects.iter().find(|r| r.id == pane_id) {
                        let pane_col = cell_col.saturating_sub(target.col).min(target.cols.saturating_sub(1));
                        let top = s.renderer.cell_origin_px(target.row, target.col, show_tabs).1;
                        let (pane_row, pane_col) = tab.pane(pane_id).map_or((cell_row.saturating_sub(target.row), pane_col), |p| {
                            let g = p.grid.lock();
                            let row = g.viewport_row(position.y - top, s.renderer.cell_size().1 as f64);
                            (row, g.logical_col(row, pane_col))
                        });
                        let end = (pane_row, pane_col);
                        if s.pointer.extend_drag {
                            if let Some(pane) = tab.pane(pane_id) {
                                pane.grid.lock().set_selection_end(pane_row, pane_col);
                                s.window.request_redraw();
                            }
                        } else if end != start || s.pointer.drag_moved {
                            s.pointer.drag_moved = true;
                            if let Some(pane) = tab.pane(pane_id) {
                                pane.grid.lock().set_selection(Some(Selection { start, end }));
                                s.window.request_redraw();
                            }
                        }
                    }
                }
            }
        } else {
            update_hovered_link(s);
        }
    }

    pub(super) fn on_mouse_input(&mut self, id: WindowId, btn_state: ElementState, button: MouseButton) {
        let waker = self.waker.clone();
        let Some(s) = self.window_mut(id) else { return };
        if s.tabs.is_empty() {
            return;
        }
        let reported = match button {
            MouseButton::Left => Some(mouse::Button::Left),
            MouseButton::Middle => Some(mouse::Button::Middle),
            MouseButton::Right => Some(mouse::Button::Right),
            _ => None,
        };
        if button == MouseButton::Left {
            if btn_state == ElementState::Released && s.pointer.scrollbar_drag.take().is_some() {
                return;
            }
            if btn_state == ElementState::Pressed && !s.history_search.is_open
                && let Some(pane_id) = scrollbar_at_mouse(s) {
                    s.pointer.scrollbar_drag = Some(pane_id);
                    drag_scrollbar(s, pane_id);
                    return;
                }
        }
        if let (Some(b), false) = (reported, s.history_search.is_open || s.pointer.selecting) {
            let action = if btn_state == ElementState::Pressed { mouse::Action::Press } else { mouse::Action::Release };
            if report_mouse(s, b, action) {
                s.pointer.mouse_button_down = (action == mouse::Action::Press).then_some(b);
                return;
            }
        }
        if btn_state == ElementState::Released {
            s.pointer.mouse_button_down = None;
        }
        if button != MouseButton::Left {
            return;
        }
        if s.history_search.is_open {
            if btn_state == ElementState::Pressed
                && let Some((x0, y0, popup_w, popup_h, visible_rows)) =
                    s.renderer.history_popup_geometry(&s.history_search)
                {
                    let show_tabs = s.tabs.len() > 1;
                    let (cols, rows) = s.renderer.grid_size_for_tabs(show_tabs);
                    if let Some((cell_row, cell_col)) =
                        s.renderer.pixel_to_cell(s.pointer.mouse_pos.0, s.pointer.mouse_pos.1, show_tabs, cols, rows)
                    {
                        let r = if show_tabs { cell_row + 1 } else { cell_row };
                        let c = cell_col;
                        if c < x0 || c >= x0 + popup_w || r < y0 || r >= y0 + popup_h {
                            s.history_search.close();
                            s.renderer.invalidate();
                            s.window.request_redraw();
                            return;
                        }
                        if r >= y0 + 3 && r < y0 + 3 + visible_rows {
                            let item_idx = s.history_search.scroll_offset + (r - (y0 + 3));
                            if item_idx < s.history_search.filtered.len() {
                                let now = std::time::Instant::now();
                                let is_double = s.history_search.selected == item_idx
                                    && s.pointer.last_click_time.is_some_and(|t| now.duration_since(t).as_millis() <= 400);
                                if is_double {
                                    let cmd_opt = s.history_search.selected_command().map(|c| c.to_string());
                                    s.history_search.close();
                                    s.renderer.invalidate();
                                    if let Some(cmd) = cmd_opt
                                        && let Some(pane) = s.tabs[s.active].active_pane() {
                                            let bracketed = pane.grid.lock().modes.bracketed_paste;
                                            write_pane_input(pane,&mut s.ai,&history_insert_payload(&cmd, true, bracketed));
                                        }
                                    s.window.request_redraw();
                                    return;
                                } else {
                                    s.history_search.selected = item_idx;
                                    s.pointer.last_click_time = Some(now);
                                    s.renderer.invalidate();
                                    s.window.request_redraw();
                                    return;
                                }
                            }
                        }
                    } else {
                        s.history_search.close();
                        s.renderer.invalidate();
                        s.window.request_redraw();
                        return;
                    }
                }
            return;
        }
        match btn_state {
            ElementState::Pressed => {
                #[cfg(target_os = "macos")]
                let cmd_pressed = s.mods.super_key();
                #[cfg(not(target_os = "macos"))]
                let cmd_pressed = s.mods.control_key() || s.mods.super_key();

                if cmd_pressed
                    && let Some((pane_id, url, is_path)) = s.pointer.hovered_link.as_ref().map(|l| (l.pane_id, l.link.url.clone(), l.link.is_path)) {
                        let cwd = s.tabs.get_mut(s.active)
                            .and_then(|t| t.pane_mut(pane_id))
                            .and_then(|p| p.cwd());
                        let cwd_path = cwd.as_deref().map(std::path::Path::new);
                        open_target(&url, is_path, cwd_path);
                        return;
                    }

                if let Some(action) = s.renderer.hit_test_tab_bar(s.pointer.mouse_pos.0, s.pointer.mouse_pos.1, &s.tab_titles) {
                    match action {
                        TabBarAction::Select(idx) => select_tab(s, idx),
                        TabBarAction::CloseTab(idx) => close_tab_at(s, idx),
                        TabBarAction::NewTab => duplicate_active_tab(s, &waker),
                    }
                    return;
                }
                let mut switched_active_pane = false;
                {
                    let target = pane_cell_at_mouse(s);
                    let tab = &mut s.tabs[s.active];
                    if let Some((clicked_id, pane_row, pane_col)) = target {

                        if tab.active_pane_id != clicked_id {
                            tab.active_pane_id = clicked_id;
                            switched_active_pane = true;
                        }
                        s.pointer.active_click_pane_id = Some(clicked_id);

                        // Shift+Click extends the pane's selection to the mouse;
                        // dragging on keeps moving its end.
                        let extended = s.mods.shift_key()
                            && tab.pane(clicked_id).is_some_and(|p| p.grid.lock().extend_selection(pane_row, pane_col));
                        s.pointer.extend_drag = extended;
                        if extended {
                            s.pointer.selecting = true;
                            s.pointer.drag_moved = true;
                            s.window.request_redraw();
                        }

                        let now = std::time::Instant::now();
                        let multi_click = !extended
                            && s.pointer.last_click_time.is_some_and(|t| now.duration_since(t).as_millis() <= 450)
                            && s.pointer.last_click_cell.is_some_and(|(lr, _)| lr == pane_row);

                        s.pointer.click_count = if multi_click {
                            (s.pointer.click_count % 3) + 1
                        } else {
                            1
                        };
                        // A Shift+Click never starts a double-click.
                        s.pointer.last_click_time = (!extended).then_some(now);
                        s.pointer.last_click_cell = Some((pane_row, pane_col));

                        if let Some(pane) = tab.pane(clicked_id).filter(|_| !extended) {
                            match s.pointer.click_count {
                                // Double-click selects a block of text; again on a
                                // selected block, the whole line.
                                2 => {
                                    let line_text = {
                                        let mut g = pane.grid.lock();
                                        if std::mem::take(&mut s.pointer.click_on_selected_block) {
                                            g.select_smart_line(pane_row, pane_col)
                                        } else {
                                            g.select_block(pane_row, pane_col)
                                        }
                                    };
                                    if !line_text.is_empty() && s.config.copy_on_select {
                                        copy_to_clipboard(&line_text);
                                    }
                                    s.pointer.selecting = false;
                                    s.pointer.drag_moved = false;
                                    s.window.request_redraw();
                                }
                                3 => {
                                    let line_text = pane.grid.lock().select_line(pane_row);
                                    if !line_text.is_empty() && s.config.copy_on_select {
                                        copy_to_clipboard(&line_text);
                                    }
                                    s.pointer.selecting = false;
                                    s.pointer.drag_moved = false;
                                    s.window.request_redraw();
                                }
                                _ => {
                                    let mut g = pane.grid.lock();
                                    s.pointer.click_on_selected_block = g.is_block_selected(pane_row, pane_col);
                                    g.set_selection(None);
                                    drop(g);
                                    s.pointer.selecting = true;
                                    s.pointer.drag_moved = false;
                                    s.window.request_redraw();
                                }
                            }
                        }
                    }
                }
                if switched_active_pane {
                    refresh_tab_titles(s);
                    s.renderer.invalidate();
                }
            }
            ElementState::Released => {
                if s.pointer.selecting {
                    s.pointer.selecting = false;
                    if s.pointer.drag_moved {
                        if let Some(pane_id) = s.pointer.active_click_pane_id
                            && let Some(pane) = s.tabs.get(s.active).and_then(|t| t.pane(pane_id)) {
                                let text = pane.grid.lock().selected_text();
                                if let Some(t) = text.filter(|_| s.config.copy_on_select) {
                                    copy_to_clipboard(&t);
                                }
                            }
                    } else if s.pointer.click_count == 1 && !s.history_search.is_open
                        && let (Some(pane_id), Some((pane_row, pane_col))) =
                            (s.pointer.active_click_pane_id, s.pointer.last_click_cell)
                            && let Some(pane) = s.tabs.get(s.active).and_then(|t| t.pane(pane_id)) {
                                let move_bytes = pane
                                    .grid
                                    .lock()
                                    .calculate_cursor_movement_to(pane_row, pane_col);
                                if let Some(bytes) = move_bytes {
                                    write_pane_input(pane,&mut s.ai,&bytes);
                                    s.window.request_redraw();
                                }
                            }
                }
            }
        }
    }

    pub(super) fn on_mouse_wheel(&mut self, id: WindowId, delta: MouseScrollDelta, phase: TouchPhase) {
        let Some(s) = self.window_mut(id) else { return };
        if s.tabs.is_empty() {
            return;
        }
        if matches!(phase, TouchPhase::Started | TouchPhase::Cancelled) {
            s.pointer.wheel_scroll.reset();
        }
        if phase == TouchPhase::Cancelled {
            return;
        }
        let target_id = pane_cell_at_mouse(s).map_or(s.tabs[s.active].active_pane_id, |(id, _, _)| id);
        let target = if s.history_search.is_open { scroll::Target::History } else { scroll::Target::Pane(target_id) };
        let (amount, unit, precise) = match delta {
            MouseScrollDelta::LineDelta(_, y) => {
                let multiplier = if s.history_search.is_open { 1.0 } else { s.config.mouse_scroll_discrete };
                (y as f64 * multiplier, 1.0, false)
            }
            MouseScrollDelta::PixelDelta(p) => {
                let multiplier = if s.history_search.is_open { 1.0 } else { s.config.mouse_scroll_precision };
                (p.y * multiplier, s.renderer.cell_size().1.max(1) as f64, true)
            }
        };
        if precise && !s.history_search.is_open && !s.search.is_open && !s.inspector.shown
            && let Some(pane) = s.tabs[s.active].pane(target_id) {
                let mut g = pane.grid.lock();
                let local = g.modes.mouse_tracking == grid::MouseTracking::Off || s.mods.shift_key() || s.mods.super_key();
                if local && !g.alt_active {
                    let changed = g.scroll_pixels(amount, unit);
                    drop(g);
                    if changed { s.window.request_redraw(); }
                    return;
                }
            }
        let lines = s.pointer.wheel_scroll.push(amount, unit, target, precise);
        if s.history_search.is_open {
            let visible_rows = s.renderer.history_popup_geometry(&s.history_search).map_or(10, |g| g.4);
            if lines > 0 {
                for _ in 0..lines.abs() {
                    s.history_search.select_prev();
                }
                s.renderer.invalidate();
                s.window.request_redraw();
            } else if lines < 0 {
                for _ in 0..lines.abs() {
                    s.history_search.select_next(visible_rows);
                }
                s.renderer.invalidate();
                s.window.request_redraw();
            }
            return;
        }
        if lines != 0 {
            let count = lines.unsigned_abs().min(20);
            let wheel = if lines > 0 { mouse::Button::WheelUp } else { mouse::Button::WheelDown };
            if report_mouse(s, wheel, mouse::Action::Press) {
                for _ in 1..count {
                    report_mouse(s, wheel, mouse::Action::Press);
                }
                return;
            }
            if alt_screen_scroll(s, lines) {
                return;
            }
            if let Some(pane) = s.tabs[s.active].pane(target_id) {
                pane.grid.lock().scroll_display(lines);
                s.window.request_redraw();
            }
        }
    }

    pub(super) fn on_focused(&mut self, id: WindowId, focused: bool) {
        let Some(s) = self.window_mut(id) else { return };
        if !focused {
            s.pointer.wheel_scroll.reset();
        }
        let was_focused = std::mem::replace(&mut s.focused, focused);
        if focused && s.occluded {
            s.occluded = false;
            let sz = s.window.inner_size();
            if sz.width > 0 && sz.height > 0 {
                s.renderer.resize(sz.width, sz.height);
            }
            s.renderer.invalidate();
        }
        if focused {
            s.renderer.set_bell_border(false);
        }
        // Secure input is system-wide; release it while another app is in front.
        if s.secure_input {
            macos::set_secure_input(focused);
        }
        if focused {
            s.quick_autohide_at = None;
            macos::set_secure_input_menu_state(s.secure_input);
        } else if was_focused && s.quick && s.config.quick_terminal_autohide {
            // A passive X11 keyboard grab emits FocusOut/FocusIn even
            // while the panel remains the active application window.
            s.quick_autohide_at = Some(std::time::Instant::now() + std::time::Duration::from_millis(50));
        }
        if let Some(tab) = s.tabs.get_mut(s.active).filter(|t| focused && t.bell) {
            tab.bell = false;
            refresh_tab_titles(s);
        }
        if let Some(tab) = s.tabs.get(s.active)
            && let Some(pane) = tab.active_pane() {
            let report_focus = pane.grid.lock().modes.focus_events;
            if report_focus {
                write_pty(pane, if focused { b"\x1b[I" } else { b"\x1b[O" });
            }
        }
        if focused {
            s.cursor_blink_epoch = std::time::Instant::now();
            let mtime = config::config_mtime();
            if mtime != s.config_mtime {
                s.requests.reload_config = true;
            }
        }
        if !s.tabs.is_empty() {
            s.window.request_redraw();
        }
    }

    pub(super) fn on_redraw_requested(&mut self, id: WindowId) {
        let waker = self.waker.clone();
        let Some(s) = self.window_mut(id) else { return };
        if s.occluded {
            return;
        }
        // Re-arm before rendering so any PTY write during the draw schedules the next frame.
        waker.rearm();
        render_active(s);
    }
}
