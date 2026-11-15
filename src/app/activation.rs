//! Activation requests change app state only on its event loop.
use super::{
    App,
    window::{create_activation_window, refresh_tab_titles, sync_size},
};
use crate::platform::{
    self,
    activation::{ActivationAction, ActivationReply, ActivationRequest},
    gui::LoopCtx,
};

impl App {
    pub(super) fn on_activation(
        &mut self,
        event_loop: &LoopCtx,
        request: ActivationRequest,
        reply: std::sync::mpsc::SyncSender<ActivationReply>,
    ) {
        let result = if !self
            .activation
            .as_ref()
            .is_some_and(|server| server.accepts(&request))
        {
            Err("activation request expired or belongs to another session".into())
        } else {
            match request.action {
                ActivationAction::NewWindow(cwd) => self.open_activation_window(event_loop, &cwd),
                ActivationAction::Pane(id) => {
                    self.activate_pane(id);
                    Ok(())
                }
            }
        };
        if reply.send(result).is_err() {
            eprintln!("litty: activation client disconnected before acknowledgement");
        }
    }

    fn open_activation_window(&mut self, event_loop: &LoopCtx, cwd: &str) -> ActivationReply {
        if !std::path::Path::new(cwd).is_dir() {
            return Err("activation directory is unavailable".into());
        }
        let state = create_activation_window(event_loop, &self.waker, cwd)
            .map_err(|e| format!("cannot create activation window: {}", e.kind()))?;
        state.window.focus_window();
        platform::activate_app();
        state.window.request_redraw();
        self.windows.push(state);
        Ok(())
    }

    fn activate_pane(&mut self, id: u64) {
        for state in &mut self.windows {
            let Some(at) = state.tabs.iter_mut().position(|tab| tab.select_pane(id)) else {
                continue;
            };
            state.active = at;
            sync_size(state);
            // As select_tab: the title/bell marker must not wait for the
            // throttled render-time refresh.
            refresh_tab_titles(state);
            state.window.restore_and_focus();
            platform::activate_app();
            state.renderer.invalidate();
            state.window.request_redraw();
            return;
        }
    }
}
