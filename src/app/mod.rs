//! Window and event-loop coordination: one `State` per window, action dispatch,
//! mouse / keyboard / IME input.

mod actions;
mod ai_bridge;
mod csd;
mod history_ui;
mod ime;
mod inspector;
mod keyboard;
mod pointer;
mod search_ui;
mod state;
mod tab_anim;
mod tab_tearoff;
mod tabs;
mod terminal_io;
mod user_event;
mod window;
mod window_event;

use actions::run_action;
use ai_bridge::handle_ai_requests;
use state::State;
use tabs::confirm_close;
use tabs::end_tab_drag;
use tabs::hide_or_exit_last_window;
use tabs::open_fresh_tab;
use terminal_io::handle_terminal_requests;
use window::apply_system_appearance;
use tab_tearoff::{adopt_dropped_tab, follow_torn_window, restore_tab, set_pointer_from_screen, tab_bar_hit_x};
use tabs::drag_tab;
use window::create_window;
use window::create_window_for_tab;
use window::cursor_blinks;
use window::place_quick_terminal;
use window::reload_config;
use window::render_active;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::terminal::grid;
use crate::render::frame_metrics;
use crate::{bench, update};
use crate::platform;
use crate::{AppEvent, Waker};

use crate::platform::gui::{EventHandler, Flow, LogicalPosition, LoopCtx, PhysicalPosition, WindowEvent, WindowId};


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
            platform::release_free_memory();
        }
    }

    /// Show the Quick Terminal (creating it on first use) or hide it when it is
    /// already in front.
    fn toggle_quick_terminal(&mut self, event_loop: &LoopCtx) {
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
                    platform::hide_app();
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

    /// Open each tab dragged out of a tab bar in a window of its own. If the window
    /// cannot be created the tab goes back to the window it came from.
    fn open_torn_off_tabs(&mut self, event_loop: &LoopCtx) {
        let torn: Vec<_> = self.windows.iter_mut().filter_map(|s| s.requests.tear_off.take()).collect();
        for torn in torn {
            let from = torn.from;
            let mut pending = Some(torn);
            match create_window_for_tab(event_loop, &self.waker, &mut pending) {
                Ok(s) => {
                    // A drag still in progress moves the new window with the pointer.
                    let id = s.window.id();
                    self.windows.push(s);
                    if let Some(drag) = self.window_mut(from).and_then(|origin| origin.pointer.tab_drag.as_mut()) {
                        drag.follow(id);
                    }
                }
                Err(e) => {
                    eprintln!("litty: failed to open window for the dragged tab: {e}");
                    if let (Some(torn), Some(origin)) = (pending, self.window_mut(from)) {
                        restore_tab(origin, torn);
                        // The tab is back in its bar: the press must not tear it off again on release.
                        end_tab_drag(origin);
                    }
                }
            }
        }
    }

    /// Move each window whose tab is being dragged out under the pointer.
    fn move_torn_windows(&mut self) {
        for i in 0..self.windows.len() {
            let Some((target, position)) = self.windows[i].requests.move_torn.take() else { continue };
            if let Some(torn) = self.window_mut(target) {
                torn.window.set_outer_position_logical(position);
            }
        }
    }

    /// Carry on each tab drag whose pointer is delivered to a window that does not hold
    /// the tab: move the holder's drag, and merge a torn-off tab into a tab bar the
    /// pointer has entered.
    fn drive_dragged_tabs(&mut self) {
        let pings: Vec<_> = self.windows.iter_mut().filter_map(|s| s.requests.drag_ping.take().map(|at| (s.window.id(), at))).collect();
        for (owner, at) in pings {
            self.drive_dragged_tab(owner, at);
        }
    }

    fn drive_dragged_tab(&mut self, owner: WindowId, at: LogicalPosition<f64>) {
        let holder = self.window_mut(owner).and_then(|o| o.pointer.drag_holder).unwrap_or(owner);
        let Some(h) = self.window_mut(holder) else {
            // The holder was closed under the drag.
            if let Some(o) = self.window_mut(owner) {
                o.pointer.drag_holder = None;
            }
            return;
        };
        let Some(drag) = h.pointer.tab_drag.as_ref() else {
            if let Some(o) = self.window_mut(owner) {
                o.pointer.drag_holder = None;
            }
            return;
        };
        let (torn, detached) = (drag.follower(), drag.is_detached());
        if holder != owner {
            set_pointer_from_screen(h, at);
            if detached {
                follow_torn_window(h);
            } else {
                drag_tab(h);
            }
        }
        if let Some(torn) = torn {
            self.merge_into_tab_bar(owner, holder, torn, at);
        }
    }

    /// The tab of the torn-off window `torn`, held by the drag of `holder`, entered the tab
    /// bar of a window under the pointer `at`: move it there, close `torn`, and carry the
    /// drag on in that window, as in Chrome.
    fn merge_into_tab_bar(&mut self, owner: WindowId, holder: WindowId, torn: WindowId, at: LogicalPosition<f64>) {
        let target = self.windows.iter().find_map(|w| {
            (w.window.id() != torn).then(|| tab_bar_hit_x(w, at).map(|x| (w.window.id(), x))).flatten()
        });
        let (Some((target, x)), Some(torn_at)) = (target, self.windows.iter().position(|w| w.window.id() == torn)) else {
            return;
        };
        let mut emptied = self.windows.remove(torn_at);
        let Some(drag) = self.window_mut(holder).and_then(|h| h.pointer.tab_drag.take()) else {
            self.windows.insert(torn_at, emptied);
            return;
        };
        let held = drag.id();
        let Some(dest) = self.window_mut(target) else { return };
        let adopted = adopt_dropped_tab(dest, &mut emptied, x, held);
        if let Some((_, grab_col)) = adopted {
            dest.pointer.tab_drag = Some(drag.resume(grab_col));
            set_pointer_from_screen(dest, at);
            drag_tab(dest);
        }
        if let Some(o) = self.window_mut(owner) {
            o.pointer.drag_holder = (target != owner && adopted.is_some()).then_some(target);
        }
        // Dropping the emptied state closes its window; its shells live on in the target.
        emptied.renderer.shrink_drawables();
        drop(emptied);
        platform::release_free_memory();
    }

    /// The button of a drag held by another window was released: the tab slides into its slot.
    fn end_forwarded_drags(&mut self) {
        let holders: Vec<_> = self.windows.iter_mut().filter_map(|s| s.requests.drag_release.take()).collect();
        for holder in holders {
            if let Some(h) = self.window_mut(holder) {
                end_tab_drag(h);
            }
        }
    }

    /// Apply what event handlers asked for: quit, open new windows, close emptied ones.
    fn process_window_requests(&mut self, event_loop: &LoopCtx) {
        if self.windows.iter_mut().any(|s| std::mem::take(&mut s.requests.reload_config)) {
            for s in &mut self.windows {
                reload_config(s);
            }
        }
        if self.windows.iter_mut().any(|s| std::mem::take(&mut s.requests.quick_terminal)) {
            self.toggle_quick_terminal(event_loop);
        }
        if self.windows.iter_mut().any(|s| std::mem::take(&mut s.requests.quit)) {
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
            let new_window = std::mem::take(&mut s.requests.new_window);
            let native_tab = std::mem::take(&mut s.requests.native_tab);
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
                        platform::add_tabbed_window(&p.window, &s.window);
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

        self.drive_dragged_tabs();
        self.open_torn_off_tabs(event_loop);
        self.move_torn_windows();
        self.end_forwarded_drags();

        while let Some(pos) = self.windows.iter().position(|s| s.requests.close) {
            if self.windows.len() > 1 {
                // Dropping the state closes the window and hangs up its shells.
                self.windows.remove(pos).renderer.shrink_drawables();
                platform::release_free_memory();
                continue;
            }
            let s = &mut self.windows[pos];
            s.requests.close = false;
            hide_or_exit_last_window(s, event_loop);
        }
    }
}

impl EventHandler<AppEvent> for App {
    fn cycle_started(&mut self, timer_fired: bool) {
        if timer_fired {
            for s in self.windows.iter().filter(|s| !s.tabs.is_empty() && !s.occluded) {
                s.window.request_redraw();
            }
        }
    }

    fn resumed(&mut self, event_loop: &LoopCtx) {
        if !self.windows.is_empty() {
            return;
        }
        let state = create_window(event_loop, &self.waker, None, false).expect("spawn initial PTY");
        self.windows.push(state);

        if self.scroll_fps_run.is_some() {
            let s = &mut self.windows[0];
            s.window.set_title("Litty — scroll FPS benchmark");
            s.window.focus_window();
            platform::activate_app();
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
            let refresh = s.window.current_monitor().and_then(|m| m.refresh_rate_millihertz);
            println!("{{\"benchmark\":\"scroll_swapchain\",\"width_px\":{},\"height_px\":{},\"scale_factor\":{},\"monitor_millihz\":{},\"custom_shaders\":{},\"warmup_ms\":1000,\"scenario_ms\":5000}}",
                size.width, size.height, s.window.scale_factor(), refresh.unwrap_or(0), s.config.custom_shaders.len());
            s.renderer.start_frame_capture(s.occluded);
            self.scroll_fps_run = Some(frame_metrics::ScrollRun::new());
            s.window.request_redraw();
            return;
        }

        let sink = crate::event::platform_sink(self.waker.proxy.clone());
        platform::register_reopen_handler(sink.clone());
        platform::install_menu();
        platform::listen_for_toggle(sink);
        window::set_quick_terminal_hotkey(self.windows[0].config.quick_terminal_hotkey.as_ref());

        let cfg = &self.windows[0].config;
        update::check_on_startup(cfg.auto_update, cfg.auto_update_channel);

        platform::release_free_memory();

        if self.bench_ram_mode {
            bench::run_ram_benchmark_thread(self.waker.proxy.clone());
        }
    }

    fn user_event(&mut self, event_loop: &LoopCtx, event: AppEvent) {
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
            AppEvent::ChildExit(exited_id) => self.on_child_exit(event_loop, exited_id),
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
            AppEvent::Reopen => self.on_reopen(),
            AppEvent::UpdateCheckResult(result) => self.on_update_check_result(result),
            AppEvent::UpdateInstallResult(result) => {
                platform::close_progress_dialog();
                match result {
                    Ok(new_ver) => {
                        platform::alert(
                            "Update Complete",
                            &format!(
                                "Litty has been successfully updated to v{new_ver}!\n\nPlease restart Litty to apply changes."
                            ),
                        );
                    }
                    Err(err) => {
                        platform::alert("Update Failed", &format!("Failed to install update:\n{err}"));
                    }
                }
            }
            AppEvent::BenchAction(cmd) => self.on_bench_action(event_loop, cmd),
        }
    }

    fn window_event(&mut self, event_loop: &LoopCtx, id: WindowId, event: WindowEvent) {
        self.handle_window_event(id, event);
        self.process_window_requests(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &LoopCtx) {
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
            event_loop.set_flow(Flow::WaitUntil(run.next));
            return;
        }
        if self.waker.is_queued() {
            event_loop.set_flow(Flow::Poll);
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
            if crate::terminal::graphics::any_animation() {
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
        event_loop.set_flow(deadline.map_or(Flow::Wait, Flow::WaitUntil));
    }
}

pub(crate) fn run(bench_ram_mode: bool, scroll_fps: bool) {
    crate::platform::gui::run(|proxy| App {
        waker: Waker { proxy, queued: Arc::new(AtomicBool::new(false)) },
        windows: Vec::new(),
        bench_ram_mode,
        scroll_fps_run: scroll_fps.then(frame_metrics::ScrollRun::new),
    });
}

#[cfg(test)]
mod latency_benchmarks;
