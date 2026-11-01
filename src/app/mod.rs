//! Window and event-loop coordination: one `State` per window, action dispatch,
//! mouse / keyboard / IME input.

mod ai_bridge;
mod ime;
mod pointer;
mod state;
mod tabs;
mod terminal_io;
mod window;

use tabs::toggle_split_zoom;
use tabs::split_active_pane;
use tabs::select_tab;
use tabs::resize_active_split;
use tabs::open_fresh_tab;
use tabs::navigate_split_directional;
use tabs::hide_or_exit_last_window;
use tabs::equalize_splits;
use tabs::duplicate_active_tab;
use tabs::cycle_split;
use tabs::confirm_close;
use tabs::close_tab_at;
use tabs::close_last_window;
use tabs::close_active_pane_or_tab;
use pointer::update_hovered_link;
use pointer::scrollbar_at_mouse;
use pointer::report_mouse;
use pointer::pane_cell_at_mouse;
use ime::handle_ime;
use pointer::drag_scrollbar;
use pointer::alt_screen_scroll;
use terminal_io::write_pty;
use terminal_io::write_pane_input;
use terminal_io::handle_terminal_requests;
use ai_bridge::handle_ai_requests;
use window::zoom;
use window::sync_size;
use window::render_active;
use window::reload_config;
use window::refresh_tab_titles;
use window::place_quick_terminal;
use window::cursor_blinks;
use window::create_window;
use window::apply_system_appearance;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::{ai, bench, config, frame_metrics, grid, keymap, macos, mouse, scroll, update};
#[cfg(target_os = "linux")]
use crate::hotkey;
use crate::{next_id, AppEvent, Waker};

use state::State;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, StartCause, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::WindowId;

use crate::grid::Selection;
use crate::paste::{history_insert_payload, paste_payload, shell_quote};
use crate::platform::clipboard::{copy_to_clipboard, paste_from_clipboard};
use crate::platform::process::{open_target, spawn_detached};
#[cfg(target_os = "linux")]
use crate::platform::process::linux_open_command;
use crate::renderer::TabBarAction;
use crate::split::{Direction, SplitDirection};

struct App {
    waker: Waker,
    /// One entry per window. On macOS a hidden window with no tabs stays here
    /// after the last one closes, so a Dock click can reopen it.
    windows: Vec<State>,
    bench_ram_mode: bool,
    scroll_fps_run: Option<frame_metrics::ScrollRun>,
}

impl App {
    fn window_mut(&mut self, id: WindowId) -> Option<&mut State> {
        self.windows.iter_mut().find(|s| s.window.id() == id)
    }

    /// Drop the hidden, tab-less window kept for the Dock once another one opens.
    fn drop_tabless_windows(&mut self) {
        let had_tabless = self.windows.iter().any(|s| s.tabs.is_empty());
        for s in self.windows.iter_mut().filter(|s| s.tabs.is_empty()) {
            s.renderer.shrink_drawables();
        }
        self.windows.retain(|w| !w.tabs.is_empty());
        if had_tabless {
            #[cfg(target_os = "macos")]
            macos::release_free_memory();
        }
    }

    /// Show the Quick Terminal (creating it on first use) or hide it when it is
    /// already in front.
    fn toggle_quick_terminal(&mut self, event_loop: &ActiveEventLoop) {
        let waker = self.waker.clone();
        if let Some(i) = self.windows.iter().position(|s| s.quick) {
            let s = &mut self.windows[i];
            if s.tabs.is_empty() {
                open_fresh_tab(s, &waker);
            } else if s.window.is_visible().unwrap_or(true) && (s.focused || s.quick_autohide_at.is_some()) {
                s.quick_autohide_at = None;
                s.window.set_visible(false);
                let others_visible = self.windows.iter().any(|w| !w.quick && !w.tabs.is_empty() && w.window.is_visible() != Some(false));
                if !others_visible {
                    macos::hide_app();
                }
                return;
            }
            s.quick_autohide_at = None;
            place_quick_terminal(&self.windows[i]);
            return;
        }
        match create_window(event_loop, &waker, None, true) {
            Ok(s) => {
                place_quick_terminal(&s);
                self.drop_tabless_windows();
                self.windows.push(s);
            }
            Err(e) => eprintln!("litty: failed to open the Quick Terminal: {e}"),
        }
    }

    /// Apply what event handlers asked for: quit, open new windows, close emptied ones.
    fn process_window_requests(&mut self, event_loop: &ActiveEventLoop) {
        if self.windows.iter_mut().any(|s| std::mem::take(&mut s.reload_config_requested)) {
            for s in &mut self.windows {
                reload_config(s);
            }
        }
        if self.windows.iter_mut().any(|s| std::mem::take(&mut s.quick_terminal_requested)) {
            self.toggle_quick_terminal(event_loop);
        }
        if self.windows.iter_mut().any(|s| std::mem::take(&mut s.quit_requested)) {
            let config = &self.windows[0].config;
            let panes = self.windows.iter().flat_map(|s| s.tabs.iter().flat_map(|t| &t.panes));
            if confirm_close(config, panes, "Litty") {
                event_loop.exit();
                return;
            }
        }
        // (cwd, window to attach the new one to as a native tab, position to cascade from)
        let mut requests = Vec::new();
        for s in &mut self.windows {
            let new_window = std::mem::take(&mut s.new_window_requested);
            let native_tab = std::mem::take(&mut s.native_tab_requested);
            if new_window || native_tab {
                let cwd = s.tabs.get_mut(s.active).and_then(|t| t.active_pane_mut()?.cwd());
                let from = (!native_tab && s.window.is_visible() != Some(false))
                    .then(|| s.window.outer_position().ok().map(|p| (p, s.window.scale_factor())))
                    .flatten();
                requests.push((cwd, native_tab.then(|| s.window.id()), from));
            }
        }
        for (cwd, parent, from) in requests {
            match create_window(event_loop, &self.waker, cwd.as_deref(), false) {
                // The hidden, tab-less window kept for the Dock is no longer needed.
                Ok(s) => {
                    if let Some(p) = parent.and_then(|id| self.windows.iter().find(|w| w.window.id() == id)) {
                        macos::add_tabbed_window(&p.window, &s.window);
                    }
                    // winit centers every new window: cascade it down-right from the
                    // one it was opened from so it does not hide exactly behind it.
                    if let Some((pos, scale)) = from {
                        let step = (28.0 * scale).round() as i32;
                        s.window.set_outer_position(PhysicalPosition::new(pos.x + step, pos.y + step));
                    }
                    self.drop_tabless_windows();
                    self.windows.push(s);
                }
                Err(e) => eprintln!("litty: failed to open window: {e}"),
            }
        }

        while let Some(pos) = self.windows.iter().position(|s| s.close_requested) {
            if self.windows.len() > 1 {
                // Dropping the state closes the window and hangs up its shells.
                self.windows.remove(pos).renderer.shrink_drawables();
                #[cfg(target_os = "macos")]
                macos::release_free_memory();
                continue;
            }
            let s = &mut self.windows[pos];
            s.close_requested = false;
            hide_or_exit_last_window(s, event_loop);
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if let StartCause::ResumeTimeReached { .. } = cause {
            for s in self.windows.iter().filter(|s| !s.tabs.is_empty() && !s.occluded) {
                s.window.request_redraw();
            }
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if !self.windows.is_empty() {
            return;
        }
        let state = create_window(event_loop, &self.waker, None, false).expect("spawn initial PTY");
        self.windows.push(state);

        if self.scroll_fps_run.is_some() {
            let s = &mut self.windows[0];
            s.window.set_title("Litty — scroll FPS benchmark");
            s.window.focus_window();
            #[cfg(target_os = "macos")]
            macos::activate_app();
            let pane = s.tabs[s.active].active_pane().unwrap();
            let mut g = pane.grid.lock();
            let mut parser = vte::Parser::new();
            for line in 0..5000 {
                let text = format!("{line:05}  Scroll rendering benchmark: alpha beta gamma 0123456789\r\n");
                for b in text.bytes() { g.feed_byte(&mut parser, b); }
            }
            g.scroll_display(2000);
            drop(g);
            let size = s.window.inner_size();
            let refresh = s.window.current_monitor().and_then(|m| m.refresh_rate_millihertz());
            println!("{{\"benchmark\":\"scroll_swapchain\",\"width_px\":{},\"height_px\":{},\"scale_factor\":{},\"monitor_millihz\":{},\"custom_shaders\":{},\"warmup_ms\":1000,\"scenario_ms\":5000}}",
                size.width, size.height, s.window.scale_factor(), refresh.unwrap_or(0), s.config.custom_shaders.len());
            s.renderer.start_frame_capture(s.occluded);
            self.scroll_fps_run = Some(frame_metrics::ScrollRun::new());
            s.window.request_redraw();
            return;
        }

        #[cfg(target_os = "macos")]
        {
            macos::register_reopen_handler(self.waker.proxy.clone());
            macos::install_menu();
            macos::set_global_hotkey(self.windows[0].config.quick_terminal_hotkey.as_ref());
        }
        #[cfg(not(target_os = "macos"))]
        {
            hotkey::listen(self.waker.proxy.clone());
            hotkey::set_global_hotkey(self.windows[0].config.quick_terminal_hotkey.as_ref());
        }

        let cfg = &self.windows[0].config;
        update::check_on_startup(cfg.auto_update, cfg.auto_update_channel);

        macos::release_free_memory();

        if self.bench_ram_mode {
            bench::run_ram_benchmark_thread(self.waker.proxy.clone());
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::AiStatus {pane_id,generation,text} => {
                for s in &mut self.windows {
                    if let Some(ai)=s.ai.get_mut(&pane_id).filter(|a|a.generation==generation && a.busy) {
                        ai.status=text;
                        handle_ai_requests(s,&self.waker);
                        s.window.request_redraw();
                        break;
                    }
                }
            }
            AppEvent::AiResult {pane_id,generation,outcome} => {
                for s in &mut self.windows {
                    if let Some(ai)=s.ai.get_mut(&pane_id).filter(|a|a.generation==generation && a.busy) {
                        ai.accept_result(generation,*outcome);
                        handle_ai_requests(s,&self.waker);
                        s.window.request_redraw();
                        break;
                    }
                }
            }
            AppEvent::WakeUp => {
                self.waker.rearm();
                for s in &mut self.windows {
                    handle_terminal_requests(s);
                    handle_ai_requests(s, &self.waker);
                    if !s.tabs.is_empty() {
                        render_active(s);
                    }
                }
            }
            AppEvent::ChildExit(exited_id) => {
                // A pane of a window that was already closed: nothing to do.
                let Some(s) = self
                    .windows
                    .iter_mut()
                    .find(|s| s.tabs.iter().any(|t| t.panes.iter().any(|p| p.id == exited_id)))
                else {
                    return;
                };
                if let Some(tab_pos) = s.tabs.iter().position(|t| t.panes.iter().any(|p| p.id == exited_id)) {
                    let tab = &mut s.tabs[tab_pos];
                    if tab.panes.len() > 1 {
                        let mut ids = Vec::new();
                        tab.layout.all_pane_ids(&mut ids);
                        let cur_idx = ids.iter().position(|&id| id == exited_id);
                        let prev_in_order = cur_idx.and_then(|idx| {
                            if idx > 0 {
                                Some(ids[idx - 1])
                            } else if ids.len() > 1 {
                                Some(ids[1])
                            } else {
                                None
                            }
                        });

                        tab.panes.retain(|p| p.id != exited_id);
                        tab.layout.remove_pane(exited_id);
                        if tab.zoomed_pane_id == Some(exited_id) {
                            tab.zoomed_pane_id = None;
                        }
                        if tab.active_pane_id == exited_id {
                            if let Some(pid) = prev_in_order.filter(|id| tab.panes.iter().any(|p| p.id == *id)) {
                                tab.active_pane_id = pid;
                            } else if let Some(p) = tab.panes.first() {
                                tab.active_pane_id = p.id;
                            }
                        }
                        s.invalidate_pane_state(exited_id);
                        sync_size(s);
                        refresh_tab_titles(s);
                        s.renderer.invalidate();
                        s.window.request_redraw();
                        #[cfg(target_os = "macos")]
                        macos::release_free_memory();
                        return;
                    }

                    s.invalidate_pane_state(exited_id);
                    s.tabs.remove(tab_pos);
                    if s.tabs.is_empty() {
                        s.close_requested = true;
                        self.process_window_requests(event_loop);
                        return;
                    }
                    if s.active >= s.tabs.len() {
                        s.active = s.tabs.len() - 1;
                    } else if tab_pos < s.active {
                        s.active -= 1;
                    }
                    sync_size(s);
                    refresh_tab_titles(s);
                    s.renderer.invalidate();
                    s.window.request_redraw();
                    #[cfg(target_os = "macos")]
                    macos::release_free_memory();
                }
            }
            AppEvent::Menu(action) => {
                // The focused window, else any window with tabs, else the hidden one.
                let target = self
                    .windows
                    .iter()
                    .position(|s| s.focused && !s.tabs.is_empty())
                    .or_else(|| self.windows.iter().position(|s| !s.tabs.is_empty()))
                    .unwrap_or(0);
                let waker = self.waker.clone();
                if let Some(s) = self.windows.get_mut(target) {
                    run_action(s, action, &waker);
                }
                self.process_window_requests(event_loop);
            }
            AppEvent::SystemAppearance { dark } => {
                if dark != grid::color_scheme_is_dark() {
                    grid::set_color_scheme_dark(dark);
                    self.windows.iter_mut().for_each(apply_system_appearance);
                }
            }
            AppEvent::Reopen => {
                // A Light/Dark switch made while another app was in front.
                if cfg!(target_os = "macos") {
                    let dark = macos::system_dark();
                    if dark != grid::color_scheme_is_dark() {
                        grid::set_color_scheme_dark(dark);
                        self.windows.iter_mut().for_each(apply_system_appearance);
                    }
                }
                // Bring the windows back; with none left, reopen the hidden one.
                let visible = self.windows.iter().position(|s| !s.tabs.is_empty());
                if let Some(s) = self.windows.get_mut(visible.unwrap_or(0)) {
                    if s.tabs.is_empty() {
                        open_fresh_tab(s, &self.waker);
                    } else {
                        let sz = s.window.inner_size();
                        s.renderer.resize(sz.width, sz.height);
                        s.window.set_visible(true);
                        s.window.focus_window();
                        #[cfg(target_os = "macos")]
                        macos::activate_app();
                        s.renderer.invalidate();
                        s.window.request_redraw();
                    }
                }
            }
            AppEvent::UpdateCheckResult(result) => {
                macos::close_progress_dialog();
                update::mark_checked();
                match result {
                    Err(err) => {
                        macos::alert("Update Check Failed", &format!("Unable to check for updates:\n{err}"));
                    }
                    Ok(release) => {
                        let current = update::Version::current();
                        if release.version <= current {
                            macos::alert(
                                "Litty",
                                &format!("You're up to date!\n\nLitty v{current} is currently the newest version available."),
                            );
                        } else {
                            let title = "Update Available";
                            let message = format!(
                                "Litty v{} is available (you have v{}).\n\nWould you like to download and install this update now?",
                                release.version, current
                            );
                            let choice = macos::alert_choice(title, &message, &["Update Now", "View Release", "Cancel"]);
                            match choice {
                                0 => {
                                    let text = format!("Downloading and installing Litty v{}…", release.version);
                                    // The result event is handled after this one, so the dialog opens before it closes.
                                    if update::start_install_update(release, self.waker.proxy.clone()) {
                                        macos::show_progress_dialog("Updating Litty…", &text);
                                    } else {
                                        macos::alert("Update In Progress", "Litty is already installing an update.");
                                    }
                                }
                                1 => {
                                    #[cfg(target_os = "macos")]
                                    let _ = spawn_detached(std::process::Command::new("/usr/bin/open").arg("--").arg(&release.html_url));
                                    #[cfg(not(target_os = "macos"))]
                                    let _ = spawn_detached(&mut linux_open_command(&release.html_url));
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            AppEvent::UpdateInstallResult(result) => {
                macos::close_progress_dialog();
                match result {
                    Ok(new_ver) => {
                        macos::alert(
                            "Update Complete",
                            &format!(
                                "Litty has been successfully updated to v{new_ver}!\n\nPlease restart Litty to apply changes."
                            ),
                        );
                    }
                    Err(err) => {
                        macos::alert("Update Failed", &format!("Failed to install update:\n{err}"));
                    }
                }
            }
            AppEvent::BenchAction(cmd) => {
                match cmd {
                    bench::BenchCommand::OpenTabInLastWindow => {
                        if let Some(s) = self.windows.last_mut() {
                            if s.tabs.is_empty() {
                                open_fresh_tab(s, &self.waker);
                            } else {
                                duplicate_active_tab(s, &self.waker);
                            }
                            self.process_window_requests(event_loop);
                        }
                    }
                    bench::BenchCommand::OpenWindow => {
                        if let Some(s) = self.windows.last_mut() {
                            s.new_window_requested = true;
                            self.process_window_requests(event_loop);
                        }
                    }
                    bench::BenchCommand::CloseLastWindow => {
                        if self.windows.len() > 1 {
                            let mut s = self.windows.remove(self.windows.len() - 1);
                            s.renderer.shrink_drawables();
                            #[cfg(target_os = "macos")]
                            macos::release_free_memory();
                        }
                    }
                    bench::BenchCommand::CloseExtraTabs => {
                        for s in &mut self.windows {
                            while s.tabs.len() > 1 {
                                s.tabs.pop();
                            }
                            s.active = 0;
                            s.renderer.shrink_drawables();
                            let sz = s.window.inner_size();
                            s.renderer.resize(sz.width, sz.height);
                            sync_size(s);
                            refresh_tab_titles(s);
                        }
                        #[cfg(target_os = "macos")]
                        macos::release_free_memory();
                    }
                    bench::BenchCommand::Exit => {
                        event_loop.exit();
                    }
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        self.handle_window_event(id, event);
        self.process_window_requests(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(run) = &mut self.scroll_fps_run {
            let now = std::time::Instant::now();
            let Some(s) = self.windows.first_mut() else { event_loop.exit(); return };
            if now.duration_since(run.start) >= std::time::Duration::from_secs(5) {
                let (label, hz, _) = frame_metrics::ScrollRun::SCENARIOS[run.scenario];
                s.renderer.finish_frame_capture(label, hz);
                run.scenario += 1;
                if run.scenario == frame_metrics::ScrollRun::SCENARIOS.len() {
                    event_loop.exit();
                    return;
                }
                run.start = now;
                run.next = now;
                run.ticks = 0;
                s.renderer.start_frame_capture(s.occluded);
            }
            if run.due(now) {
                let (_, _, pixels) = frame_metrics::ScrollRun::SCENARIOS[run.scenario];
                if let Some(pane) = s.tabs[s.active].active_pane() {
                    let direction = if run.ticks % 240 < 120 { 1.0 } else { -1.0 };
                    pane.grid.lock().scroll_pixels(direction * pixels, s.renderer.cell_size().1 as f64);
                }
                s.window.request_redraw();
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(run.next));
            return;
        }
        if self.waker.is_queued() {
            event_loop.set_control_flow(ControlFlow::Poll);
            return;
        }
        let mut deadline: Option<std::time::Instant> = None;
        let now = std::time::Instant::now();
        for s in &mut self.windows {
            if let Some(hide_at) = s.quick_autohide_at {
                if now >= hide_at {
                    s.quick_autohide_at = None;
                    if !s.focused && s.config.quick_terminal_autohide {
                        s.window.set_visible(false);
                    }
                } else {
                    deadline = Some(deadline.map_or(hide_at, |d| d.min(hide_at)));
                }
            }
            if let Some(next_prune) = s.renderer.prune_idle_images(now) {
                deadline = Some(deadline.map_or(next_prune, |d| d.min(next_prune)));
            }
            if s.sync_pending {
                let sync = now + std::time::Duration::from_millis(16);
                deadline = Some(deadline.map_or(sync, |d| d.min(sync)));
            }
            // An animated custom shader: about 60 frames per second.
            let always = s.config.custom_shader_animation == Some(true);
            if !s.tabs.is_empty() && s.renderer.animating() && (s.focused || always) {
                let frame = now + std::time::Duration::from_millis(16);
                deadline = Some(deadline.map_or(frame, |d| d.min(frame)));
            }
            // Kitty animations on screen: wake for their next frame.
            if crate::graphics::any_animation() {
                let panes = s.tabs.get(s.active).into_iter().flat_map(|t| &t.panes);
                if let Some(next) = panes.filter_map(|p| p.grid.lock().next_image_frame(now)).min() {
                    deadline = Some(deadline.map_or(next, |d| d.min(next)));
                }
            }
            if !s.tabs.is_empty() && s.focused && cursor_blinks(s) {
                let elapsed_ms = s.cursor_blink_epoch.elapsed().as_millis();
                let next_boundary_ms = ((elapsed_ms / 500) + 1) * 500;
                let blink = now + std::time::Duration::from_millis((next_boundary_ms - elapsed_ms) as u64);
                deadline = Some(deadline.map_or(blink, |d| d.min(blink)));
            }
        }
        event_loop.set_control_flow(deadline.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}

impl App {
    fn handle_window_event(&mut self, id: WindowId, event: WindowEvent) {
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

            WindowEvent::CursorMoved { position, .. } => {
                s.mouse_pos = (position.x, position.y);
                if s.tabs.is_empty() {
                    return;
                }
                if s.mouse_hidden {
                    s.window.set_cursor_visible(true);
                    s.mouse_hidden = false;
                }
                if s.inspector {
                    s.window.request_redraw();
                }
                s.renderer.set_mouse(position.x, position.y);
                if let Some(pane_id) = s.scrollbar_drag {
                    drag_scrollbar(s, pane_id);
                    return;
                }
                if !s.selecting && !s.history_search.is_open {
                    let button = s.mouse_button_down.unwrap_or(mouse::Button::None);
                    if report_mouse(s, button, mouse::Action::Motion) {
                        return;
                    }
                }
                if s.selecting {
                    if let (Some(start), Some(pane_id)) = (s.last_click_cell, s.active_click_pane_id) {
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
                                if s.extend_drag {
                                    if let Some(pane) = tab.pane(pane_id) {
                                        pane.grid.lock().set_selection_end(pane_row, pane_col);
                                        s.window.request_redraw();
                                    }
                                } else if end != start || s.drag_moved {
                                    s.drag_moved = true;
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

            WindowEvent::MouseInput { state: btn_state, button, .. } => {
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
                    if btn_state == ElementState::Released && s.scrollbar_drag.take().is_some() {
                        return;
                    }
                    if btn_state == ElementState::Pressed && !s.history_search.is_open
                        && let Some(pane_id) = scrollbar_at_mouse(s) {
                            s.scrollbar_drag = Some(pane_id);
                            drag_scrollbar(s, pane_id);
                            return;
                        }
                }
                if let (Some(b), false) = (reported, s.history_search.is_open || s.selecting) {
                    let action = if btn_state == ElementState::Pressed { mouse::Action::Press } else { mouse::Action::Release };
                    if report_mouse(s, b, action) {
                        s.mouse_button_down = (action == mouse::Action::Press).then_some(b);
                        return;
                    }
                }
                if btn_state == ElementState::Released {
                    s.mouse_button_down = None;
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
                                s.renderer.pixel_to_cell(s.mouse_pos.0, s.mouse_pos.1, show_tabs, cols, rows)
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
                                            && s.last_click_time.is_some_and(|t| now.duration_since(t).as_millis() <= 400);
                                        if is_double {
                                            let cmd_opt = s.history_search.selected_command().map(|c| c.to_string());
                                            s.history_search.close();
                                            s.renderer.invalidate();
                                            if let Some(cmd) = cmd_opt
                                                && let Some(pane) = s.tabs[s.active].active_pane() {
                                                    let bracketed = pane.grid.lock().bracketed_paste;
                                                    write_pane_input(pane,&mut s.ai,&history_insert_payload(&cmd, true, bracketed));
                                                }
                                            s.window.request_redraw();
                                            return;
                                        } else {
                                            s.history_search.selected = item_idx;
                                            s.last_click_time = Some(now);
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
                            && let Some((pane_id, url, is_path)) = s.hovered_link.as_ref().map(|l| (l.pane_id, l.link.url.clone(), l.link.is_path)) {
                                let cwd = s.tabs.get_mut(s.active)
                                    .and_then(|t| t.pane_mut(pane_id))
                                    .and_then(|p| p.cwd());
                                let cwd_path = cwd.as_deref().map(std::path::Path::new);
                                open_target(&url, is_path, cwd_path);
                                return;
                            }

                        if let Some(action) = s.renderer.hit_test_tab_bar(s.mouse_pos.0, s.mouse_pos.1, &s.tab_titles) {
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
                                s.active_click_pane_id = Some(clicked_id);

                                // Shift+Click extends the pane's selection to the mouse;
                                // dragging on keeps moving its end.
                                let extended = s.mods.shift_key()
                                    && tab.pane(clicked_id).is_some_and(|p| p.grid.lock().extend_selection(pane_row, pane_col));
                                s.extend_drag = extended;
                                if extended {
                                    s.selecting = true;
                                    s.drag_moved = true;
                                    s.window.request_redraw();
                                }

                                let now = std::time::Instant::now();
                                let multi_click = !extended
                                    && s.last_click_time.is_some_and(|t| now.duration_since(t).as_millis() <= 450)
                                    && s.last_click_cell.is_some_and(|(lr, _)| lr == pane_row);

                                s.click_count = if multi_click {
                                    (s.click_count % 3) + 1
                                } else {
                                    1
                                };
                                // A Shift+Click never starts a double-click.
                                s.last_click_time = (!extended).then_some(now);
                                s.last_click_cell = Some((pane_row, pane_col));

                                if let Some(pane) = tab.pane(clicked_id).filter(|_| !extended) {
                                    match s.click_count {
                                        // Double-click selects a block of text; again on a
                                        // selected block, the whole line.
                                        2 => {
                                            let line_text = {
                                                let mut g = pane.grid.lock();
                                                if std::mem::take(&mut s.click_on_selected_block) {
                                                    g.select_smart_line(pane_row, pane_col)
                                                } else {
                                                    g.select_block(pane_row, pane_col)
                                                }
                                            };
                                            if !line_text.is_empty() && s.config.copy_on_select {
                                                copy_to_clipboard(&line_text);
                                            }
                                            s.selecting = false;
                                            s.drag_moved = false;
                                            s.window.request_redraw();
                                        }
                                        3 => {
                                            let line_text = pane.grid.lock().select_line(pane_row);
                                            if !line_text.is_empty() && s.config.copy_on_select {
                                                copy_to_clipboard(&line_text);
                                            }
                                            s.selecting = false;
                                            s.drag_moved = false;
                                            s.window.request_redraw();
                                        }
                                        _ => {
                                            let mut g = pane.grid.lock();
                                            s.click_on_selected_block = g.is_block_selected(pane_row, pane_col);
                                            g.set_selection(None);
                                            drop(g);
                                            s.selecting = true;
                                            s.drag_moved = false;
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
                        if s.selecting {
                            s.selecting = false;
                            if s.drag_moved {
                                if let Some(pane_id) = s.active_click_pane_id
                                    && let Some(pane) = s.tabs.get(s.active).and_then(|t| t.pane(pane_id)) {
                                        let text = pane.grid.lock().selected_text();
                                        if let Some(t) = text.filter(|_| s.config.copy_on_select) {
                                            copy_to_clipboard(&t);
                                        }
                                    }
                            } else if s.click_count == 1 && !s.history_search.is_open
                                && let (Some(pane_id), Some((pane_row, pane_col))) =
                                    (s.active_click_pane_id, s.last_click_cell)
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

            WindowEvent::MouseWheel { delta, phase, .. } => {
                if s.tabs.is_empty() {
                    return;
                }
                if matches!(phase, TouchPhase::Started | TouchPhase::Cancelled) {
                    s.wheel_scroll.reset();
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
                if precise && !s.history_search.is_open && !s.search.is_open && !s.inspector
                    && let Some(pane) = s.tabs[s.active].pane(target_id) {
                        let mut g = pane.grid.lock();
                        let local = g.mouse_tracking == grid::MouseTracking::Off || s.mods.shift_key() || s.mods.super_key();
                        if local && !g.alt_active {
                            let changed = g.scroll_pixels(amount, unit);
                            drop(g);
                            if changed { s.window.request_redraw(); }
                            return;
                        }
                    }
                let lines = s.wheel_scroll.push(amount, unit, target, precise);
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

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed && s.renderer.set_bell_border(false) {
                    s.window.request_redraw();
                }
                if s.ime_preedit.is_empty() {
                    handle_key(s, &event, &waker);
                }
            }

            WindowEvent::Ime(ime) => handle_ime(s, ime),

            // Dropped files paste their shell-quoted paths, separated by spaces.
            WindowEvent::DroppedFile(path) => {
                if let Some(tab) = s.tabs.get(s.active)
                    && let Some(pane) = tab.active_pane() {
                        let bracketed = pane.grid.lock().bracketed_paste;
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

            WindowEvent::Focused(focused) => {
                if !focused {
                    s.wheel_scroll.reset();
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
                    let report_focus = pane.grid.lock().focus_events;
                    if report_focus {
                        write_pty(pane, if focused { b"\x1b[I" } else { b"\x1b[O" });
                    }
                }
                if focused {
                    s.cursor_blink_epoch = std::time::Instant::now();
                    let mtime = config::config_mtime();
                    if mtime != s.config_mtime {
                        s.reload_config_requested = true;
                    }
                }
                if !s.tabs.is_empty() {
                    s.window.request_redraw();
                }
            }

            WindowEvent::RedrawRequested => {
                if s.occluded {
                    return;
                }
                // Re-arm before rendering so any PTY write during the draw schedules the next frame.
                waker.rearm();
                render_active(s);
            }

            _ => {}
        }
    }
}

fn handle_key(
    s: &mut State,
    ev: &KeyEvent,
    waker: &Waker,
) {
    let m = keymap::Mods {
        ctrl: s.mods.control_key(),
        alt: s.mods.alt_key(),
        shift: s.mods.shift_key(),
        sup: s.mods.super_key(),
    };
    if ev.state != ElementState::Pressed {
        handle_key_release(s, ev, m);
        return;
    }
    s.locks = keymap::infer_locks(s.locks, &ev.logical_key, ev.location, m.shift);

    let is_modifier = keymap::is_modifier_key(&ev.logical_key);
    if !is_modifier && s.config.mouse_hide_while_typing && !s.mouse_hidden {
        s.window.set_cursor_visible(false);
        s.mouse_hidden = true;
    }
    s.cursor_blink_epoch = std::time::Instant::now();

    if s.history_search.is_open && handle_history_search_key(s, &ev.logical_key, m) {
        consume_key(s, ev);
        return;
    }

    let alt_screen = s.tabs.get(s.active).and_then(|t| t.active_pane()).is_some_and(|p| p.grid.lock().alt_active);
    if s.search.is_open {
        // Keys belong to the search box; shortcuts it does not consume still work,
        // but nothing reaches the shell.
        consume_key(s, ev);
        if !handle_search_key(s, &ev.logical_key, m)
            && let Some(action) = keymap::resolve(keymap::PLATFORM, &ev.logical_key, m, alt_screen) {
                run_action(s, action, waker);
            }
        return;
    }
    // User `keybind` entries come first; the last matching one wins.
    let unshifted_key = ev.key_without_modifiers();
    let user_binding = s
        .config
        .keybinds
        .iter()
        .rev()
        .find(|(t, _)| keymap::trigger_matches(t, &unshifted_key, m))
        .map(|(_, b)| b.clone());
    let builtin = match user_binding {
        Some(keymap::Binding::Action(action)) => {
            run_action(s, action, waker);
            consume_key(s, ev);
            return;
        }
        Some(keymap::Binding::Text(bytes)) => {
            if let Some(tab) = s.tabs.get(s.active)
                && let Some(pane) = tab.active_pane() {
                    write_pane_input(pane,&mut s.ai,&bytes);
                }
            consume_key(s, ev);
            return;
        }
        Some(keymap::Binding::Ignore) => {
            consume_key(s, ev);
            return;
        }
        Some(keymap::Binding::Unbind) => None,
        None => keymap::resolve(keymap::PLATFORM, &ev.logical_key, m, alt_screen),
    };
    if let Some(action) = builtin
        && run_action(s, action, waker) {
            consume_key(s, ev);
            return;
        }
    if s.tabs.is_empty() {
        consume_key(s, ev);
        return;
    }

    let Some(pane) = s.tabs[s.active].active_pane() else {
        consume_key(s, ev);
        return;
    };
    let pane_id = pane.id;
    if s.ai.get(&pane_id).is_some_and(|a| a.busy) {
        let cancel = keymap::ai_cancel(&ev.logical_key, m);
        if cancel {
            let ai = s.ai.get_mut(&pane_id).unwrap();
            ai.cancel_request();
            if ai.authorized.take().is_some() {
                // A canceled bridge must not leave input waiting for a worker
                // that was never started (including a malformed OSC payload).
                {
                    let mut g = pane.grid.lock();
                    g.ai_armed = false;
                    g.ai_request = None;
                }
                ai.submitted = None;
                ai.result = Some(ai::Outcome {
                    session: ai.session.take().unwrap_or_default(),
                    command: None,
                    text: "Cancelled".into(),
                    error: true,
                    literal: false,
                });
            }
        } else {
            let (app_cursor, kitty_flags, modify_other_keys) = {
                let g = pane.grid.lock();
                (g.app_cursor, g.kitty_keyboard_flags(), g.modify_other_keys)
            };
            if let Some(bytes) =
                encode_terminal_key(ev, m, s.locks, app_cursor, kitty_flags, modify_other_keys)
            {
                s.ai.get_mut(&pane_id).unwrap().enqueue(&bytes);
            }
        }
        consume_key(s, ev);
        if cancel {
            handle_ai_requests(s, waker);
            s.window.request_redraw();
        }
        return;
    }
    let is_enter =
        ev.logical_key == Key::Named(NamedKey::Enter) && !m.alt && !m.ctrl && !m.shift && !m.sup;
    if is_enter {
        let (input, ready, epoch) = {
            let g = pane.grid.lock();
            (g.full_prompt_input(), g.prompt_ready(), g.prompt_epoch)
        };
        if ready
            && pane.grid.lock().ai_available
            && let Some(query) = ai::query(&input)
        {
            let ai = s.ai.entry(pane_id).or_default();
            ai.submitted(&input);
            ai.awaiting = false;
            ai.authorized = Some(query.into());
            ai.generation = next_id();
            ai.cancel = Arc::new(AtomicBool::new(false));
            ai.busy = true;
            ai.prompt_epoch = epoch;
            let direct = pane.grid.lock().ai_direct
                || input.starts_with(char::is_whitespace)
                || query.len() > 4096;
            if direct {
                // Clear Readline/Reedline's buffer and submit an empty line. The
                // natural language is never parsed or executed by the shell.
                {
                    let mut g = pane.grid.lock();
                    g.ai_track = false;
                    g.ai_request = Some(query.into());
                }
                ai.display_query = Some(input.clone());
                s.history_search.add_session_command(&input);
                write_pty(pane, b"\x05\x15\r");
                consume_key(s, ev);
                return;
            }
            {
                let mut g = pane.grid.lock();
                g.ai_track = false;
                g.ai_armed = true;
            }
        } else if let Some(ai) = s.ai.get_mut(&pane_id) {
            if input.trim().is_empty() {
                ai.submitted("");
                pane.grid.lock().ai_track = false;
            }
            ai.prompt_epoch = epoch;
        }
    } else if m.ctrl
        && matches!(&ev.logical_key,Key::Character(ch) if ch.eq_ignore_ascii_case("c"))
        && let Some(ai) = s.ai.get_mut(&pane_id)
    {
        ai.suggestion = None;
        ai.awaiting = false;
        ai.executed = None;
        pane.grid.lock().ai_track = false;
    }

    let (app_cursor, kitty_flags, modify_other_keys, ctrl_u_bytes) = {
        let mut g = pane.grid.lock();
        let app_cursor = g.app_cursor;
        let kitty_flags = g.kitty_keyboard_flags();
        let modify_other_keys = g.modify_other_keys;

        // A lone modifier (Cmd before C, Shift before a click) keeps the selection
        // and the scrollback position; only real keys snap back to the prompt.
        if !is_modifier {
            g.reset_display();
        }

        let ctrl_u_bytes = match &ev.logical_key {
            // Ctrl+U at a prompt: delete back to the prompt only (vim keeps its half-page scroll).
            Key::Character(ch) if m == (keymap::Mods { ctrl: true, ..Default::default() }) && ch.eq_ignore_ascii_case("u") => {
                if !g.alt_active {
                    g.calculate_delete_before_cursor()
                } else {
                    None
                }
            }
            Key::Named(NamedKey::Enter) => {
                let cmd = if g.alt_active { String::new() } else { g.current_prompt_input() };
                if !cmd.is_empty() {
                    s.history_search.add_session_command(&cmd);
                }
                None
            }
            _ => None,
        };

        (app_cursor, kitty_flags, modify_other_keys, ctrl_u_bytes)
    };

    // Unbound Cmd/Super combinations only reach applications that speak the Kitty
    // protocol (which can encode Super); a shell would just see the bare key.
    if m.sup && !is_modifier && kitty_flags == 0 {
        consume_key(s, ev);
        return;
    }

    if let Some(bytes) = ctrl_u_bytes {
        write_pty(pane, &bytes);
        s.window.request_redraw();
        return;
    }

    let bytes=encode_terminal_key(ev,m,s.locks,app_cursor,kitty_flags,modify_other_keys);
    if let Some(bytes) = &bytes {
        write_pty(pane, bytes);
    }
    if s.inspector {
        s.inspector_key = describe_key(&ev.logical_key, m, bytes.as_deref());
        s.window.request_redraw();
    }
}

/// Encode queued input with the same protocol and modifiers as immediate input.
fn encode_terminal_key(
    ev: &KeyEvent,
    m: keymap::Mods,
    locks: u8,
    app_cursor: bool,
    kitty_flags: u8,
    modify_other_keys: u8,
) -> Option<Vec<u8>> {
    if kitty_flags != 0 {
        let kind = if ev.repeat {
            keymap::KeyKind::Repeat
        } else {
            keymap::KeyKind::Press
        };
        let unshifted = ev.key_without_modifiers();
        keymap::encode_kitty(
            keymap::PLATFORM,
            &kitty_key(ev, &unshifted, kind, locks),
            m,
            kitty_flags,
            app_cursor,
        )
    } else if modify_other_keys != 0 {
        let text = ev.text.as_deref();
        keymap::encode_modify_other_keys(
            keymap::PLATFORM,
            &ev.logical_key,
            text,
            m,
            modify_other_keys,
            app_cursor,
        )
    } else {
        keymap::encode(
            keymap::PLATFORM,
            &ev.logical_key,
            ev.text.as_deref(),
            m,
            app_cursor,
        )
    }
}

/// `Ctrl+Shift+A → "\e[97;6u"` for the inspector.
fn describe_key(key: &Key, m: keymap::Mods, bytes: Option<&[u8]>) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (on, name) in [(m.ctrl, "Ctrl+"), (m.alt, "Alt+"), (m.shift, "Shift+"), (m.sup, "Super+")] {
        if on {
            out.push_str(name);
        }
    }
    match key {
        Key::Character(c) => out.push_str(c),
        other => {
            let _ = write!(out, "{other:?}");
        }
    }
    out.push_str(" → ");
    match bytes {
        None => out.push_str("(nothing)"),
        Some(bytes) => {
            out.push('"');
            for &b in bytes {
                match b {
                    0x1b => out.push_str("\\e"),
                    0x20..=0x7e => out.push(b as char),
                    _ => {
                        let _ = write!(out, "\\x{b:02x}");
                    }
                }
            }
            out.push('"');
        }
    }
    out
}

/// What the terminal inspector shows about the active pane.
fn inspector_lines(s: &mut State) -> Vec<String> {
    let mouse_cell = pane_cell_at_mouse(s);
    let Some(tab) = s.tabs.get_mut(s.active) else { return Vec::new() };
    let Some(pane) = tab.active_pane_mut() else { return Vec::new() };
    let mouse_cell = mouse_cell.filter(|&(id, _, _)| id == pane.id);
    let running = pane.has_running_process();
    let cwd = pane.cwd().unwrap_or_default();
    let g = pane.grid.lock();
    let on = |b: bool| if b { "on" } else { "off" };
    let hex = |c: u32| format!("#{:06x}", c >> 8);
    let (history, offset) = g.scroll_position();
    let cursor = g.cursor_pos().map_or("hidden".to_string(), |(c, r)| format!("row {r} col {c}"));
    let shape = match g.cursor_shape {
        0 => "default",
        1 | 2 => "block",
        3 | 4 => "underline",
        _ => "bar",
    };
    let cell = mouse_cell
        .and_then(|(_, r, c)| g.describe_cell(r, c).map(|d| format!("{r},{c}  {d}")))
        .unwrap_or_else(|| "(move the mouse over this pane)".to_string());
    vec![
        "Terminal Inspector".to_string(),
        format!("Grid      {}x{}  scrollback {history} lines, {offset} up", g.cols, g.rows),
        format!("Cursor    {cursor}  {shape}  {}", if g.cursor_visible { "visible" } else { "invisible" }),
        format!(
            "Screen    {}  app-cursor {}  bracketed-paste {}  sync {}",
            if g.alt_active { "alternate" } else { "main" },
            on(g.app_cursor),
            on(g.bracketed_paste),
            on(g.sync_output)
        ),
        format!(
            "Mouse     {:?}  sgr {}  pixels {}  focus {}  alt-scroll {}",
            g.mouse_tracking,
            on(g.mouse_sgr),
            on(g.mouse_sgr_pixels),
            on(g.focus_events),
            on(g.alt_scroll)
        ),
        format!("Keyboard  kitty flags {}  modifyOtherKeys {}", g.kitty_keyboard_flags(), g.modify_other_keys),
        format!("Colors    fg {}  bg {}", hex(g.fg_color()), hex(g.bg_color())),
        format!("Process   {}  cwd {cwd}", if running { "running" } else { "shell at prompt" }),
        format!("Title     {:?}", g.title),
        format!("Cell      {cell}"),
        format!("Last key  {}", s.inspector_key),
    ]
}

/// Remember that Litty handled this key press itself.
fn consume_key(s: &mut State, ev: &KeyEvent) {
    if !s.consumed_keys.contains(&ev.physical_key) {
        // A release can be lost (e.g. focus moved away mid-press); never grow unbounded.
        if s.consumed_keys.len() >= 16 {
            s.consumed_keys.clear();
        }
        s.consumed_keys.push(ev.physical_key);
    }
}

/// Key releases only matter to applications that asked for Kitty event reporting.
fn handle_key_release(s: &mut State, ev: &KeyEvent, m: keymap::Mods) {
    if let Some(i) = s.consumed_keys.iter().position(|k| *k == ev.physical_key) {
        s.consumed_keys.swap_remove(i);
        return;
    }
    if s.tabs.get(s.active).and_then(|t|t.active_pane()).is_some_and(|p|s.ai.get(&p.id).is_some_and(|a|a.busy)) {return;}
    if s.tabs.is_empty() || s.search.is_open || s.history_search.is_open {
        return;
    }
    let Some(pane) = s.tabs[s.active].active_pane() else { return; };
    let (app_cursor, kitty_flags) = {
        let g = pane.grid.lock();
        (g.app_cursor, g.kitty_keyboard_flags())
    };
    if kitty_flags & keymap::KITTY_REPORT_EVENTS == 0 {
        return;
    }
    let unshifted = ev.key_without_modifiers();
    let key = kitty_key(ev, &unshifted, keymap::KeyKind::Release, s.locks);
    if let Some(bytes) = keymap::encode_kitty(keymap::PLATFORM, &key, m, kitty_flags, app_cursor) {
        write_pty(pane, &bytes);
    }
}

fn kitty_key<'a>(ev: &'a KeyEvent, unshifted: &'a Key, kind: keymap::KeyKind, inferred: u8) -> keymap::KittyKey<'a> {
    keymap::KittyKey {
        key: &ev.logical_key,
        unshifted,
        physical: match ev.physical_key {
            PhysicalKey::Code(code) => Some(code),
            PhysicalKey::Unidentified(_) => None,
        },
        location: ev.location,
        text: ev.text.as_deref(),
        kind,
        // winit has no lock state: macOS reads Caps Lock from AppKit (and has no
        // Num Lock), elsewhere it is inferred from the keys typed.
        locks: if cfg!(target_os = "macos") {
            if macos::caps_lock() { keymap::KITTY_CAPS_LOCK } else { 0 }
        } else {
            inferred
        },
    }
}

/// Keys while the history search popup is open. Returns `false` for keys the popup
/// does not consume (e.g. `Cmd+V`), which then go through normal shortcut handling.
fn handle_history_search_key(s: &mut State, key: &Key, m: keymap::Mods) -> bool {
    let visible_rows = s.renderer.history_popup_geometry(&s.history_search).map_or(10, |g| g.4);
    let hs = &mut s.history_search;
    match key {
        Key::Named(NamedKey::Escape) => hs.close(),
        Key::Named(k @ (NamedKey::Enter | NamedKey::Tab)) => {
            let cmd = hs.selected_command().map(str::to_string);
            hs.close();
            if let (Some(cmd), Some(tab)) = (cmd, s.tabs.get(s.active)) {
                // Enter runs it, Tab leaves it for editing.
                if let Some(pane) = tab.active_pane() {
                    let bracketed = pane.grid.lock().bracketed_paste;
                    write_pane_input(pane,&mut s.ai,&history_insert_payload(&cmd, *k == NamedKey::Enter, bracketed));
                }
            }
        }
        Key::Named(NamedKey::Delete) => {
            hs.delete_selected();
        }
        Key::Named(NamedKey::ArrowDown) => hs.select_next(visible_rows),
        Key::Named(NamedKey::ArrowUp) => hs.select_prev(),
        Key::Named(NamedKey::PageDown) => hs.page_down(visible_rows),
        Key::Named(NamedKey::PageUp) => hs.page_up(visible_rows),
        Key::Named(NamedKey::Home) => hs.select_first(),
        Key::Named(NamedKey::End) => hs.select_last(visible_rows),
        Key::Named(NamedKey::Backspace) => hs.backspace(),
        Key::Character(ch) if m.ctrl => match ch.as_str() {
            // fzf bindings: Ctrl+R toggles relevance/recency sorting, Ctrl+D deletes.
            "r" => hs.toggle_sort(),
            "d" => {
                hs.delete_selected();
            }
            "n" | "j" => hs.select_next(visible_rows),
            "p" | "k" => hs.select_prev(),
            "c" | "g" => hs.close(),
            "u" | "w" => hs.clear_query(),
            _ => return true,
        },
        Key::Character(ch) if m.alt && ch.eq_ignore_ascii_case("s") => hs.toggle_sort(),
        Key::Character(ch) if !m.sup && !m.alt => ch.chars().for_each(|c| hs.append_char(c)),
        Key::Character(_) => return false,
        _ => return true,
    }
    s.renderer.invalidate();
    s.window.request_redraw();
    true
}

/// Keys while the find box is open. Returns `false` for keys it does not consume
/// (e.g. `Cmd+V`, `Ctrl+Shift+V`, `Cmd+1`), which go through shortcut handling.
fn handle_search_key(s: &mut State, key: &Key, m: keymap::Mods) -> bool {
    let plain_ctrl = m.ctrl && !m.shift && !m.alt && !m.sup;
    match key {
        Key::Named(NamedKey::Escape) => close_search(s),
        // Enter searches upwards (older output), Shift+Enter downwards.
        Key::Named(NamedKey::Enter) => search_step(s, !m.shift),
        Key::Named(NamedKey::ArrowUp) => search_step(s, true),
        Key::Named(NamedKey::ArrowDown) => search_step(s, false),
        Key::Named(NamedKey::Backspace) => {
            if m.ctrl || m.alt || m.sup {
                s.search.clear_query();
            } else {
                s.search.backspace();
            }
            refresh_search(s);
        }
        Key::Named(NamedKey::Space) if !m.ctrl && !m.sup => {
            s.search.append_char(' ');
            refresh_search(s);
        }
        // Cmd+G / Cmd+Shift+G (Super on Linux): find next / previous.
        Key::Character(ch) if m.sup && !m.ctrl && !m.alt && ch.eq_ignore_ascii_case("g") => search_step(s, !m.shift),
        Key::Character(ch) if plain_ctrl || (m.alt && !m.ctrl && !m.sup && ch.eq_ignore_ascii_case("r")) => {
            match ch.to_lowercase().as_str() {
                "r" => {
                    s.search.toggle_regex();
                    refresh_search(s);
                }
                "u" | "w" => {
                    s.search.clear_query();
                    refresh_search(s);
                }
                "c" | "g" => close_search(s),
                "p" => search_step(s, true),
                "n" => search_step(s, false),
                _ => {}
            }
        }
        Key::Character(ch) if !m.sup && !m.ctrl => {
            ch.chars().for_each(|c| s.search.append_char(c));
            refresh_search(s);
        }
        Key::Character(_) => return false,
        _ => {}
    }
    true
}

/// Re-run the find query on the active pane, moving the matches off a pane that
/// lost focus.
fn refresh_search(s: &mut State) {
    let Some(tab) = s.tabs.get(s.active) else { return };
    let Some(pane) = tab.active_pane() else { return };
    if let Some(old) = s.search.pane_id.filter(|&id| id != pane.id)
        && let Some(p) = s.tabs.iter().flat_map(|t| &t.panes).find(|p| p.id == old) {
            p.grid.lock().clear_search();
        }
    s.search.pane_id = Some(pane.id);
    // Copy the text under the lock, match outside it.
    let matches = match s.search.compile() {
        Some(re) => {
            let text = pane.grid.lock().search_text();
            text.find(&re)
        }
        None => Vec::new(),
    };
    {
        let mut g = pane.grid.lock();
        if matches.is_empty() {
            g.clear_search();
        } else {
            g.set_search_matches(matches);
        }
        (s.search.current, s.search.total) = g.search_status();
    }
    s.renderer.invalidate();
    s.window.request_redraw();
}

fn search_step(s: &mut State, up: bool) {
    if let Some(tab) = s.tabs.get(s.active)
        && let Some(pane) = tab.active_pane() {
            pane.grid.lock().search_step(up);
            s.window.request_redraw();
        }
}

fn close_search(s: &mut State) {
    if let Some(id) = s.search.pane_id
        && let Some(p) = s.tabs.iter().flat_map(|t| &t.panes).find(|p| p.id == id) {
            p.grid.lock().clear_search();
        }
    s.search.close();
    s.renderer.invalidate();
    s.window.request_redraw();
}

/// Execute a shortcut. Returns `false` when it does not apply in the current state
/// (e.g. switching tabs with a single tab), so the key falls through to the shell.
fn run_action(
    s: &mut State,
    action: keymap::Action,
    waker: &Waker,
) -> bool {
    use keymap::Action as A;

    // These work even with no open tab (macOS keeps running without windows).
    match action {
        A::Quit => s.quit_requested = true,
        A::Minimize => s.window.set_minimized(true),
        A::OpenConfig => config::open_config_in_editor(),
        A::ReloadConfig => s.reload_config_requested = true,
        A::NewWindow => s.new_window_requested = true,
        A::ToggleQuickTerminal => s.quick_terminal_requested = true,
        A::NewTab if s.tabs.is_empty() => open_fresh_tab(s, waker),
        A::NewTab if cfg!(target_os = "macos") && s.config.macos_native_tabs => s.native_tab_requested = true,
        A::NewTab => duplicate_active_tab(s, waker),
        A::CheckUpdate => {
            macos::show_progress_dialog("Checking for Updates…", "Checking for new releases of Litty…");
            update::start_interactive_check(waker.proxy.clone(), s.config.auto_update_channel);
        }
        _ if s.tabs.is_empty() => {}
        _ => return run_tab_action(s, action, waker),
    }
    true
}

fn run_tab_action(
    s: &mut State,
    action: keymap::Action,
    waker: &Waker,
) -> bool {
    use keymap::Action as A;
    let tabs = s.tabs.len();
    let panes = s.tabs[s.active].panes.len();
    match action {
        A::ClearScreen => {
            if let Some(pane) = s.tabs[s.active].active_pane() {
                pane.grid.lock().clear_screen_and_history();
                write_pane_input(pane,&mut s.ai,b"\x0c");
                s.window.request_redraw();
            }
        }
        A::Find => {
            s.history_search.close();
            s.search.open();
            refresh_search(s);
        }
        A::HistorySearch => {
            if s.search.is_open {
                close_search(s);
            }
            let initial = s.tabs[s.active].active_pane().map(|p| p.grid.lock().current_prompt_input());
            let initial_str = initial.as_deref().unwrap_or("");
            s.history_search.open((!initial_str.is_empty()).then_some(initial_str));
            s.renderer.invalidate();
            s.window.request_redraw();
        }
        A::CloseTab => close_active_pane_or_tab(s),
        A::SplitRight => split_active_pane(s, SplitDirection::Vertical, waker),
        A::SplitDown => split_active_pane(s, SplitDirection::Horizontal, waker),
        A::EqualizeSplits => equalize_splits(s),
        A::ToggleSplitZoom => toggle_split_zoom(s),
        A::JumpToPrompt(delta) => {
            let Some(pane) = s.tabs[s.active].active_pane() else {
                return false;
            };
            if !pane.grid.lock().jump_to_prompt(delta) {
                return false;
            }
            s.window.request_redraw();
        }
        A::SelectLastOutput => {
            let text = s.tabs[s.active].active_pane().and_then(|p| p.grid.lock().select_last_output());
            match text {
                Some(ref t) => copy_to_clipboard(t),
                None => return false,
            }
            s.window.request_redraw();
        }
        A::ToggleInspector => {
            s.inspector = !s.inspector;
            s.renderer.invalidate();
            s.window.request_redraw();
        }
        A::ToggleSecureInput => {
            s.secure_input = !s.secure_input;
            if s.focused {
                macos::set_secure_input(s.secure_input);
            }
            macos::set_secure_input_menu_state(s.secure_input);
            // Force the window title (and its 🔒) to be rewritten.
            s.tab_titles.clear();
            refresh_tab_titles(s);
        }
        A::SelectAll => {
            if let Some(pane) = s.tabs[s.active].active_pane() {
                pane.grid.lock().select_all();
                s.window.request_redraw();
            }
        }
        A::Copy => {
            if let Some(text) = s.tabs[s.active].active_pane().and_then(|p| p.grid.lock().selected_text()) {
                copy_to_clipboard(&text);
            }
        }
        A::Paste if s.search.is_open => {
            if let Some(text) = paste_from_clipboard() {
                s.search.append_str(&text);
                refresh_search(s);
            }
        }
        A::Paste => {
            if let Some(text) = paste_from_clipboard().filter(|t| !t.is_empty())
                && let Some(pane) = s.tabs[s.active].active_pane() {
                    let bracketed = {
                        let mut g = pane.grid.lock();
                        g.reset_display();
                        g.bracketed_paste
                    };
                    let has_newlines = text.contains('\n') || text.contains('\r');
                    if !bracketed && has_newlines {
                        let line_count = text.lines().count().max(2);
                        let preview = text
                            .lines()
                            .take(3)
                            .map(|l| if l.len() > 60 { format!("{}…", &l[..60]) } else { l.to_string() })
                            .collect::<Vec<_>>()
                            .join("\n");
                        let msg = format!(
                            "Pasting {line_count} lines into a shell without bracketed paste may execute commands immediately.\n\nPreview:\n{preview}\n\nDo you want to paste anyway?"
                        );
                        if !macos::confirm("Confirm Multi-Line Paste", &msg, "Paste") {
                            return true;
                        }
                    }
                    write_pane_input(pane,&mut s.ai,&paste_payload(&text, bracketed));
                    s.window.request_redraw();
                }
        }
        // A single in-window tab: move between native macOS tabs, if any.
        A::PrevTab if tabs < 2 => return macos::select_native_tab(&s.window, -1, None),
        A::NextTab if tabs < 2 => return macos::select_native_tab(&s.window, 1, None),
        A::SelectTab(n) if tabs < 2 => return macos::select_native_tab(&s.window, 0, Some(n)),
        A::PrevTab => select_tab(s, (s.active + tabs - 1) % tabs),
        A::NextTab => select_tab(s, (s.active + 1) % tabs),
        A::SelectTab(n) => select_tab(s, if n == 8 { tabs - 1 } else { n }),
        A::PrevSplit | A::NextSplit if panes < 2 => return false,
        A::PrevSplit => cycle_split(s, false),
        A::NextSplit => cycle_split(s, true),
        A::Navigate(dir) => {
            // At the left/right edge, move to the neighbouring tab instead.
            if !navigate_split_directional(s, dir) && tabs > 1 {
                match dir {
                    Direction::Left => select_tab(s, (s.active + tabs - 1) % tabs),
                    Direction::Right => select_tab(s, (s.active + 1) % tabs),
                    _ => {}
                }
            }
        }
        A::ResizeSplit(delta) => resize_active_split(s, delta),
        A::ZoomIn => zoom(s, 1.0),
        A::ZoomOut => zoom(s, -1.0),
        A::ZoomReset => zoom(s, 0.0),
        A::ScrollPageUp | A::ScrollPageDown => {
            if let Some(pane) = s.tabs[s.active].active_pane() {
                let mut g = pane.grid.lock();
                let page = g.rows.saturating_sub(1).max(1) as isize;
                g.scroll_display(if action == A::ScrollPageUp { page } else { -page });
                drop(g);
                s.window.request_redraw();
            }
        }
        A::Quit | A::Minimize | A::OpenConfig | A::ReloadConfig | A::NewTab | A::NewWindow | A::CheckUpdate | A::ToggleQuickTerminal => {}
    }
    true
}

pub(crate) fn run(bench_ram_mode: bool, scroll_fps: bool) {
    let event_loop = EventLoop::<AppEvent>::with_user_event().build().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App {
        waker: Waker { proxy: event_loop.create_proxy(), queued: Arc::new(AtomicBool::new(false)) },
        windows: Vec::new(),
        bench_ram_mode,
        scroll_fps_run: scroll_fps.then(frame_metrics::ScrollRun::new),
    };
    event_loop.run_app(&mut app).expect("run app");
}

#[cfg(test)]
mod latency_benchmarks;
