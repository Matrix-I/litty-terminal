//! Main-thread integration fixture; only the OS launch-classification boundary
//! is injected. Menu dispatch, window creation, PTY and activation are real.
use super::*;
use std::os::unix::fs::FileTypeExt;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

struct Lifecycle {
    app: App,
    endpoint: Option<PathBuf>,
}

impl Lifecycle {
    fn assert_benchmarks_do_not_advertise(&mut self) {
        self.app.bench_ram_mode = true;
        self.app.start_activation_service();
        assert!(self.app.activation.is_none(), "RAM benchmark advertised activation");
        self.app.bench_ram_mode = false;
        self.app.scroll_fps_run = Some(frame_metrics::ScrollRun::new());
        self.app.start_activation_service();
        assert!(self.app.activation.is_none(), "scroll benchmark advertised activation");
        self.app.scroll_fps_run = None;
        for benchmark in [
            gui_bench::GuiBench::FirstFrame(gui_bench::FirstFrameRun::new(0)),
            gui_bench::GuiBench::InputLatency(gui_bench::InputLatencyRun::new(1)),
            gui_bench::GuiBench::Throughput,
        ] {
            self.app.gui_bench = Some(benchmark);
            self.app.start_activation_service();
            assert!(self.app.activation.is_none(), "GUI benchmark advertised activation");
        }
        self.app.gui_bench = None;
    }

    /// Deliver one admitted request to the app as the activation worker would.
    fn activate(&mut self, ctx: &LoopCtx, id: [u8; 16], action: platform::activation::ActivationAction)
        -> platform::activation::ActivationReply {
        let origin = self.app.activation.as_ref().expect("activation service").origin().to_owned();
        let session_hex = origin.rsplit('-').next().expect("origin session");
        let mut session = [0; 16];
        for (i, byte) in session.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&session_hex[i * 2..i * 2 + 2], 16).expect("session hex");
        }
        let (reply, result) = mpsc::sync_channel(1);
        self.app.user_event(ctx, AppEvent::Activation {
            request: platform::activation::ActivationRequest {
                id, session,
                // This already-admitted fixture request must not expire mid-assertion.
                expires_ns: u64::MAX,
                action,
            }, reply,
        });
        result.try_recv().expect("activation acknowledgement")
    }

    /// A click on a pane in a hidden tab selects that tab and pane, clears a
    /// zoom on another pane and the bell, and refreshes the window title.
    fn assert_pane_activation_reveals_hidden_tab(&mut self, ctx: &LoopCtx) {
        use platform::{MenuCommand, PlatformEvent};
        let target = self.app.windows[0].tabs[0].active_pane().expect("first tab pane").id;
        gui_bench::set_pane_program(vec!["/bin/cat".into()]);
        self.app.user_event(ctx, PlatformEvent::Menu(MenuCommand::SplitRight).into());
        let s = &mut self.app.windows[0];
        assert_eq!(s.tabs[0].panes.len(), 2, "split did not add a pane");
        assert_ne!(s.tabs[0].active_pane_id, target);
        tabs::toggle_split_zoom(s);
        assert!(s.tabs[0].zoomed_pane_id.is_some_and(|id| id != target));
        gui_bench::set_pane_program(vec!["/bin/cat".into()]);
        self.app.user_event(ctx, PlatformEvent::Menu(MenuCommand::NewTab).into());
        let s = &mut self.app.windows[0];
        // A render titles the window after the newly active tab.
        window::refresh_tab_titles(s);
        assert_eq!((s.tabs.len(), s.active, s.titled_tab), (2, 1, 1), "new tab is not the titled active tab");
        s.tabs[0].bell = true;

        let result = self.activate(ctx, [2; 16], platform::activation::ActivationAction::Pane(target));
        assert_eq!(result, Ok(()));
        let s = &self.app.windows[0];
        assert_eq!(s.active, 0, "hidden tab was not selected");
        assert_eq!(s.tabs[0].active_pane_id, target, "clicked pane was not selected");
        assert_eq!(s.tabs[0].zoomed_pane_id, None, "zoom on another pane hides the clicked pane");
        assert!(!s.tabs[0].bell, "bell marker survived selecting the tab");
        assert_eq!(s.titled_tab, 0, "window title still names the previously active tab");

        let windows = self.app.windows.len();
        let missing = format!("/tmp/litty-missing-{}", std::process::id());
        let result = self.activate(ctx, [3; 16], platform::activation::ActivationAction::NewWindow(missing));
        assert!(result.is_err(), "missing directory must be an explicit error");
        assert_eq!(self.app.windows.len(), windows, "missing directory must not open a fallback window");
    }
}

impl EventHandler<AppEvent> for Lifecycle {
    fn cycle_started(&mut self, _: bool) {}
    fn resumed(&mut self, ctx: &LoopCtx) {
        self.assert_benchmarks_do_not_advertise();
        // A stable real PTY child avoids executing the user's interactive shell.
        gui_bench::set_pane_program(vec!["/bin/cat".into()]);
        self.app.resumed(ctx);
        assert_eq!(self.app.windows.len(), 1, "notification cold launch must create one fresh window");
        let pane = self.app.windows[0].tabs[0].active_pane().expect("cold-start pane");
        let stale_id = pane.id;
        assert!(pane.pty.pid > 0);
        // SAFETY: signal zero checks existence without sending a signal.
        assert_eq!(unsafe { libc::kill(pane.pty.pid, 0) }, 0, "cold-start PTY is not alive");
        let origin = self.app.activation.as_ref().expect("cold-start service").origin().to_owned();
        self.app.resumed(ctx);
        assert_eq!(self.app.windows.len(), 1, "repeated resume duplicated the cold window");
        self.app.windows.clear();

        let result = self.activate(ctx, [1; 16], platform::activation::ActivationAction::Pane(stale_id));
        assert_eq!(result, Ok(()), "stale-pane acknowledgement");
        assert!(self.app.windows.is_empty(), "stale pane click created a replacement window");
        let cold_endpoint = PathBuf::from(format!("/tmp/litty-a1-{}", unsafe { libc::geteuid() }))
            .join(origin).join("s");
        self.app.activation.take();
        assert!(!cold_endpoint.exists(), "cold service endpoint survived shutdown");

        // Retain coverage of the windowless menu starting a missing service.
        gui_bench::set_pane_program(vec!["/bin/cat".into()]);
        self.app.user_event(ctx, platform::PlatformEvent::Menu(platform::MenuCommand::NewWindow).into());
        assert_eq!(self.app.windows.len(), 1, "windowless menu did not create a window");
        let pane = self.app.windows[0].tabs[0].active_pane().expect("real pane");
        assert!(pane.pty.pid > 0);
        // SAFETY: signal zero checks existence without sending a signal.
        assert_eq!(unsafe { libc::kill(pane.pty.pid, 0) }, 0, "PTY child is not alive");
        self.assert_pane_activation_reveals_hidden_tab(ctx);
        let service = self.app.activation.as_ref().expect("menu window did not start activation service");
        // Inspect only this process's endpoint; never discover or reuse another
        // user's running instance during an integration test.
        let endpoint = PathBuf::from(format!("/tmp/litty-a1-{}", unsafe { libc::geteuid() }))
            .join(service.origin()).join("s");
        assert!(std::fs::symlink_metadata(&endpoint).expect("advertised endpoint").file_type().is_socket());
        let origin = service.origin().to_owned();
        self.app.start_activation_service();
        assert_eq!(self.app.activation.as_ref().unwrap().origin(), origin, "service restarted");
        self.endpoint = Some(endpoint);
        ctx.exit();
    }
    fn user_event(&mut self, _: &LoopCtx, _: AppEvent) {}
    fn window_event(&mut self, _: &LoopCtx, _: WindowId, _: WindowEvent) {}
    fn about_to_wait(&mut self, _: &LoopCtx) {}
}

impl Drop for Lifecycle {
    fn drop(&mut self) {
        self.app.windows.clear();
        self.app.activation.take();
        if let Some(endpoint) = &self.endpoint {
            assert!(!endpoint.exists(), "activation endpoint survived shutdown");
        }
    }
}

pub(crate) fn run() {
    // Bound hangs in AppKit/driver initialization without sleeps or polling.
    let (done, completion) = mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if matches!(completion.recv_timeout(Duration::from_secs(45)), Err(mpsc::RecvTimeoutError::Timeout)) {
            eprintln!("FAIL: native activation lifecycle exceeded its deadline");
            std::process::exit(2);
        }
    });
    platform::capture_startup_activation();
    platform::macos::set_notification_startup_for_test(Some(true));
    platform::gui::run(|proxy| Lifecycle {
        app: App {
            waker: Waker::new(proxy), activation: None, _notification_router: None,
            startup_cwd: None, windows: Vec::new(), bench_ram_mode: false,
            scroll_fps_run: None, gui_bench: None, gui_bench_result: None,
        },
        endpoint: None,
    });
    platform::macos::set_notification_startup_for_test(None);
    done.send(()).expect("watchdog completion");
    watchdog.join().expect("watchdog");
    println!("PASS: one cold-start window/PTY, stale pane ignored, menu window/service, hidden-tab pane activation, missing-dir rejection, benchmark exclusions, shutdown cleanup");
}
