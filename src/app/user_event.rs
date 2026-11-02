//! User event handlers split out of `ApplicationHandler::user_event`.

use super::tabs::duplicate_active_tab;
use super::tabs::open_fresh_tab;
use super::window::apply_system_appearance;
use super::window::refresh_tab_titles;
use super::window::sync_size;
use super::App;

use crate::terminal::grid;
use crate::{bench, update};
use crate::platform::macos;

use winit::event_loop::ActiveEventLoop;

use crate::platform::process::spawn_detached;
#[cfg(target_os = "linux")]
use crate::platform::process::linux_open_command;

impl App {
    pub(super) fn on_child_exit(&mut self, event_loop: &ActiveEventLoop, exited_id: u64) {
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
                s.requests.close = true;
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

    pub(super) fn on_reopen(&mut self) {
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

    pub(super) fn on_update_check_result(&mut self, result: Result<update::ReleaseInfo, String>) {
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

    pub(super) fn on_bench_action(&mut self, event_loop: &ActiveEventLoop, cmd: crate::bench::BenchCommand) {
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
                    s.requests.new_window = true;
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
