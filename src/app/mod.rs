//! Window and event-loop coordination: one `State` per window, action dispatch,
//! mouse / keyboard / IME input.

mod actions;
mod ai_bridge;
mod ime;
mod inspector;
mod keyboard;
mod pointer;
mod search_ui;
mod state;
mod tabs;
mod terminal_io;
mod user_event;
mod window;
mod window_event;

use actions::run_action;
use ai_bridge::handle_ai_requests;
use state::State;
use tabs::confirm_close;
use tabs::hide_or_exit_last_window;
use tabs::open_fresh_tab;
use terminal_io::handle_terminal_requests;
use window::apply_system_appearance;
use window::create_window;
use window::cursor_blinks;
use window::place_quick_terminal;
use window::reload_config;
use window::render_active;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::{bench, frame_metrics, grid, macos, update};
#[cfg(target_os = "linux")]
use crate::hotkey;
use crate::{AppEvent, Waker};

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;


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
            AppEvent::BenchAction(cmd) => self.on_bench_action(event_loop, cmd),
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
