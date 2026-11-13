//! Window and event-loop coordination: one `State` per window, action dispatch,
//! mouse / keyboard / IME input.

mod actions;
mod ai_bridge;
mod csd;
mod gui_bench;
mod history_ui;
mod ime;
mod inspector;
mod keyboard;
mod pointer;
mod prewarm;
mod search_ui;
mod state;
mod tab_anim;
mod tab_tearoff;
mod tabs;
mod terminal_io;
mod user_event;
mod window;
mod window_event;
mod winch;

use actions::run_action;
use ai_bridge::handle_ai_requests;
use state::State;
use tabs::confirm_close;
use tabs::end_tab_drag;
use terminal_io::handle_terminal_requests;
use window::apply_system_appearance;
use tab_tearoff::{adopt_dropped_tab, follow_torn_window, restore_tab, set_pointer_from_screen, tab_bar_hit_x};
use tabs::drag_tab;
use window::create_window;
use window::create_window_for_tab;
use window::cursor_blinks;
use window::reload_config;
use window::sync_all_tabs;

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
    /// One entry per window. Empty on macOS after the last window closes: the
    /// app stays in the Dock and a Dock click or menu action opens a new one.
    windows: Vec<State>,
    bench_ram_mode: bool,
    scroll_fps_run: Option<frame_metrics::ScrollRun>,
    gui_bench: Option<gui_bench::GuiBench>,
    /// A finished in-app benchmark's report, printed when the loop next idles.
    gui_bench_result: Option<Result<String, String>>,
}

impl App {
    fn window_mut(&mut self, id: WindowId) -> Option<&mut State> {
        self.windows.iter_mut().find(|s| s.window.id() == id)
    }

    /// Open a fresh window with one tab in the home directory and bring it to the front.
    fn open_window(&mut self, event_loop: &LoopCtx) {
        match create_window(event_loop, &self.waker, None) {
            Ok(s) => {
                s.window.focus_window();
                platform::activate_app();
                s.window.request_redraw();
                self.windows.push(s);
            }
            Err(e) => eprintln!("litty: failed to open window: {e}"),
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
                    s.window.request_redraw();
                    let dragging = self.window_mut(from)
                        .and_then(|origin| origin.pointer.tab_drag.as_mut())
                        .map(|drag| { drag.follow(id); true })
                        .unwrap_or(false);
                    if !dragging {
                        s.window.focus_window();
                        platform::activate_app();
                    }
                    self.windows.push(s);
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
                h.window.focus_window();
                platform::activate_app();
                h.window.request_redraw();
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
            match create_window(event_loop, &self.waker, cwd.as_deref()) {
                Ok(s) => {
                    if let Some(p) = parent.and_then(|id| self.windows.iter().find(|w| w.window.id() == id)) {
                        platform::add_tabbed_window(&p.window, &s.window);
                    }
                    // Every new window opens centered: cascade it down-right from the
                    // one it was opened from so it does not hide exactly behind it.
                    if let Some((pos, scale)) = from {
                        let step = (28.0 * scale).round() as i32;
                        s.window.set_outer_position(PhysicalPosition::new(pos.x + step, pos.y + step));
                    }
                    s.window.focus_window();
                    platform::activate_app();
                    s.window.request_redraw();
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
            if self.windows.len() == 1 && !cfg!(target_os = "macos") {
                event_loop.exit();
                return;
            }
            // Dropping the state closes the window and hangs up its shells. On
            // macOS the last window goes too, releasing its renderer, glyph atlas
            // and pipelines while the app waits in the Dock.
            let mut s = self.windows.remove(pos);
            if s.secure_input && s.focused {
                platform::set_secure_input(false);
            }
            s.renderer.shrink_drawables();
            drop(s);
            platform::release_free_memory();
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
        let state = create_window(event_loop, &self.waker, None).expect("spawn initial PTY");
        self.windows.push(state);

        if let Some(gui_bench::GuiBench::FirstFrame(run)) = &mut self.gui_bench {
            run.window_created();
            platform::install_menu();
            self.windows[0].window.request_redraw();
            return;
        }

        if matches!(self.gui_bench, Some(gui_bench::GuiBench::InputLatency(_) | gui_bench::GuiBench::Throughput)) {
            let s = &mut self.windows[0];
            s.window.set_title("Litty — benchmark");
            s.window.focus_window();
            platform::activate_app();
            s.window.request_redraw();
            return;
        }

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

        platform::register_reopen_handler(crate::event::platform_sink(self.waker.proxy.clone()));
        platform::install_menu();
        // Once per process, not per window: the icon stays set. A RAM run skips
        // it so it measures what bundled users get (their Dock reads Litty.icns).
        if !self.bench_ram_mode {
            platform::set_dock_icon();
        }

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
                    s.history_search.poll();
                    handle_terminal_requests(s);
                    handle_ai_requests(s, &self.waker);
                    // An occluded window draws again once it is shown (`Occluded(false)`);
                    // one that may not draw yet (Wayland frame pacing) asks for a redraw.
                    if !s.tabs.is_empty() && !s.occluded && s.window.may_redraw() {
                        gui_bench::render_observed(s, &mut self.gui_bench, &mut self.gui_bench_result);
                    }
                }
            }
            AppEvent::ChildExit(exited_id) => self.on_child_exit(event_loop, exited_id),
            AppEvent::Menu(action) if self.windows.is_empty() => self.run_windowless_action(event_loop, action),
            AppEvent::Menu(action) => {
                // The focused window, else the first one.
                let target = self.windows.iter().position(|s| s.focused).unwrap_or(0);
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
            AppEvent::Reopen => self.on_reopen(event_loop),
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
        if let Some(result) = self.gui_bench_result.take() {
            finish_gui_bench(event_loop, result);
            return;
        }
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
        if let Some(gui_bench::GuiBench::InputLatency(run)) = &mut self.gui_bench {
            let Some(s) = self.windows.first_mut() else { event_loop.exit(); return };
            match run.tick(s, &self.waker, now) {
                gui_bench::Step::Continue(at) => deadline = at,
                gui_bench::Step::Done(result) => {
                    finish_gui_bench(event_loop, result);
                    return;
                }
            }
        }
        for s in &mut self.windows {
            if let Some(next_prune) = s.renderer.prune_idle_images(now) {
                deadline = Some(deadline.map_or(next_prune, |d| d.min(next_prune)));
            }
            if s.background_resize.take_due(now) {
                sync_all_tabs(s);
            }
            if let Some(settle) = s.background_resize.deadline() {
                deadline = Some(deadline.map_or(settle, |d| d.min(settle)));
            }
            if let Some(panes) = s.winch.take_due(now) {
                for pane in s.tabs.iter().flat_map(|t| &t.panes).filter(|p| panes.contains(&p.id)) {
                    pane.pty.resignal_winch();
                }
            }
            if let Some(settle) = s.winch.deadline() {
                deadline = Some(deadline.map_or(settle, |d| d.min(settle)));
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
    run_app(bench_ram_mode, scroll_fps.then(frame_metrics::ScrollRun::new), None);
}

/// `+bench-input-latency`: the first pane runs `echo_child` (argv) instead of a
/// shell; the app types `samples` keys into it and prints one JSON report line.
pub(crate) fn run_input_latency(samples: usize, echo_child: Vec<String>) {
    gui_bench::set_pane_program(echo_child);
    run_app(false, None, Some(gui_bench::GuiBench::InputLatency(gui_bench::InputLatencyRun::new(samples))));
}

/// `+bench-pty-throughput-gui`: the first pane runs `stream_child` (argv), which
/// writes one generated stream as fast as the app takes it.
pub(crate) fn run_throughput_gui(stream_child: Vec<String>) {
    gui_bench::set_pane_program(stream_child);
    run_app(false, None, Some(gui_bench::GuiBench::Throughput));
}

/// `+bench-first-frame-child`: a normal startup (shell included, update check
/// skipped) that prints its stage times once the first frame is complete.
pub(crate) fn run_first_frame_child(spawn_ns: u64) {
    run_app(false, None, Some(gui_bench::GuiBench::FirstFrame(gui_bench::FirstFrameRun::new(spawn_ns))));
}

/// Every GUI run: start the window-independent setup, then the event loop.
fn run_app(bench_ram_mode: bool, scroll_fps_run: Option<frame_metrics::ScrollRun>, gui_bench: Option<gui_bench::GuiBench>) {
    prewarm::start();
    crate::platform::gui::run(|proxy| App {
        waker: Waker { proxy, queued: Arc::new(AtomicBool::new(false)) },
        windows: Vec::new(),
        bench_ram_mode,
        scroll_fps_run,
        gui_bench,
        gui_bench_result: None,
    });
}

/// Print a benchmark's report line and stop the event loop; a failed run
/// prints why and exits with status 1 right away (dropping the PTY children).
fn finish_gui_bench(event_loop: &LoopCtx, result: Result<String, String>) {
    match result {
        Ok(line) => println!("{line}"),
        Err(e) => {
            eprintln!("litty: benchmark failed: {e}");
            std::process::exit(1);
        }
    }
    event_loop.exit();
}

#[cfg(test)]
mod latency_benchmarks;
