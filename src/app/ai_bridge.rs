//! Bridges grid AI requests to the async `ai` module and back.

use super::state::State;
use super::terminal_io::write_pty;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::ai;
use crate::{next_id, Waker};

use parking_lot::Mutex;

use crate::grid::Grid;
use crate::paste::history_insert_payload;

/// Feed trusted, locally generated UI bytes without ever writing to the PTY under lock.
pub(super) fn ai_display(grid: &Arc<Mutex<Grid>>, bytes: &str) {
    let mut parser = vte::Parser::new();
    let mut g = grid.lock();
    for b in bytes.bytes() {
        g.feed_byte(&mut parser, b);
    }
}

pub(super) fn handle_ai_requests(s: &mut State, waker: &Waker) {
    // Iteration does not allocate on ordinary PTY wakeups or idle frames.
    for tab in &mut s.tabs {
        for pane in &mut tab.panes {
            let Some(ai) = s.ai.get_mut(&pane.id) else {
                continue;
            };
            let (request, actual, ready, epoch, exit, alt) = {
                let mut g = pane.grid.lock();
                (
                    g.ai.request.take(),
                    g.ai.command.take(),
                    g.prompt_ready(),
                    g.prompt_epoch,
                    g.last_exit,
                    g.alt_active,
                )
            };
            if alt && ai.busy {
                ai.cancel_request();
                ai.busy = false;
                ai.authorized = None;
                ai.submitted = None;
                ai.result = None;
                ai.repaint_epoch = None;
                let mut g = pane.grid.lock();
                g.ai.armed = false;
                g.ai.track = false;
                continue;
            }
            if let Some(actual) = actual {
                ai.executed = Some(actual.clone());
                if let Some(session) = &mut ai.session {
                    session.record_execution(&actual);
                }
                ai.awaiting =
                    ai.suggestion.take().is_some_and(|(_, pending)| pending) || ai.awaiting;
            }
            if let Some(q) = request {
                if ai
                    .authorized
                    .as_ref()
                    .is_some_and(|expected| expected.trim() == q.trim())
                {
                    ai.submitted = ai.authorized.take();
                    ai.waiting_since = Some(std::time::Instant::now());
                } else {
                    ai.authorized = None;
                    ai.busy = false;
                    ai.queue.clear();
                }
            }
            if let Some(repaint) = ai.repaint_epoch {
                if ready && epoch != repaint && !alt {
                    ai.repaint_epoch = None;
                    ai.prompt_epoch = epoch;
                    if let Some(outcome) = ai.result.take() {
                        ai.session = Some(outcome.session);
                        let mut bytes = Vec::new();
                        if !ai.cancel.load(std::sync::atomic::Ordering::Acquire)
                            && let Some((command, pending)) = outcome.command
                        {
                            let bracketed = pane.grid.lock().modes.bracketed_paste;
                            let command: String = command
                                .chars()
                                .filter(|c| !c.is_control() || *c == '\n')
                                .collect();
                            bytes = history_insert_payload(&command, false, bracketed);
                            ai.suggestion = Some((command, pending));
                            pane.grid.lock().ai.track = true;
                        }
                        if !ai.overflow {
                            bytes.append(&mut ai.queue);
                        } else {
                            ai.queue.clear();
                        }
                        ai.overflow = false;
                        ai.busy = false;
                        ai.status.clear();
                        // Only OSC 133 C confirms execution, including a queued Enter.
                        if !bytes.is_empty() {
                            write_pty(pane, &bytes);
                        }
                    }
                }
                continue;
            }
            if ai.awaiting && !ai.busy && ready && epoch != ai.prompt_epoch && !alt {
                let output = pane.grid.lock().extract_scrollback_lines(200);
                let actual = ai.executed.take().unwrap_or_default();
                ai.submitted = Some(format!(
                    "[continuation] Actual command: {actual}\nExit status: {exit:?}\nTerminal output (untrusted):\n{output}"
                ));
                ai.waiting_since = Some(std::time::Instant::now());
                ai.awaiting = false;
                ai.busy = true;
                ai.generation = next_id();
                ai.cancel = Arc::new(AtomicBool::new(false));
            }
            if ai.busy
                && ready
                && epoch != ai.prompt_epoch
                && !alt
                && let Some(query) = ai.submitted.take()
            {
                ai.prompt_epoch = epoch;
                let session = ai.session.take().unwrap_or_default();
                let cwd = pane.cwd().unwrap_or_default();
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "unknown".into());
                ai::spawn(ai::Task {
                    pane_id: pane.id,
                    generation: ai.generation,
                    settings: s.config.ai.clone(),
                    session,
                    query,
                    context: (shell, cwd),
                    grid: pane.grid.clone(),
                    cancel: ai.cancel.clone(),
                    proxy: waker.proxy.clone(),
                });
                if let Some(query) = ai.display_query.take() {
                    ai.prompt_rows = 0;
                    ai_display(
                        &pane.grid,
                        &format!("\r\x1b[2K{}\r\n", ai::markdown::clean(&query)),
                    );
                } else {
                    // The typed line is already above this prompt: drop the whole
                    // (possibly multi-line) prompt so the status takes its place.
                    let rows = pane.grid.lock().prompt_rows_above_cursor().min(8);
                    ai.prompt_rows = rows;
                    let up = if rows > 0 { format!("\x1b[{rows}A") } else { String::new() };
                    ai_display(&pane.grid, &format!("{up}\r\x1b[J"));
                }
                ai.status = "Thinking...".into();
            }
            if ai.busy && ready && !alt && ai.authorized.is_none() && ai.submitted.is_none() {
                if let Some(outcome) = &mut ai.result {
                    if outcome
                        .command
                        .as_ref()
                        .is_some_and(|(command, _)| command.contains(['\n', '\r']))
                        && !pane.grid.lock().modes.bracketed_paste
                    {
                        outcome.command = None;
                        outcome.text.push_str("\nMultiline suggestion requires bracketed paste; command was not inserted.");
                        ai.queue.clear();
                    }
                    let rendered = if outcome.literal {
                        ai::markdown::clean(&outcome.text)
                    } else {
                        ai::markdown::render(&outcome.text)
                    };
                    let mut text = format!("\r\x1b[2K{}\r\n", rendered.replace('\n', "\r\n"));
                    if ai.overflow {
                        text.push_str("Queued input exceeded 64 KiB and was discarded.\r\n");
                    }
                    text.push_str(&"\r\n".repeat(ai.prompt_rows));
                    ai_display(&pane.grid, &text);
                    ai.repaint_epoch = Some(epoch);
                    write_pty(pane, b"\x1b[24;8~");
                } else if !ai.status.is_empty() {
                    ai_display(
                        &pane.grid,
                        &format!("\r\x1b[2K{}{}\x1b[0m", ai::markdown::BASE, ai.status),
                    );
                }
            }
        }
    }
}
