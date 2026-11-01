//! Explicit, credentialed checks against Anthropic; never part of normal tests.
use super::*;
use std::time::{Duration, Instant};

fn invoke(
    c: &config::Config,
    session: Session,
    query: &str,
    grid: &Arc<Mutex<Grid>>,
    timeout: Duration,
) -> (Outcome, bool, Duration) {
    let cancel = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let compacting = AtomicBool::new(false);
    let (watch_cancel, watch_done) = (cancel.clone(), done.clone());
    let start = Instant::now();
    let watch = std::thread::spawn(move || {
        while !watch_done.load(Ordering::Acquire) {
            if start.elapsed() >= timeout {
                watch_cancel.store(true, Ordering::Release);
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    let out = run(
        || Ok(c.clone()),
        session,
        query,
        &("zsh".into(), "/tmp/litty-ai-live-fixture".into()),
        grid,
        &cancel,
        &|status| {
            if status == "Compacting..." {
                compacting.store(true, Ordering::Release);
            }
        },
    );
    done.store(true, Ordering::Release);
    watch.join().expect("live-test watchdog");
    (out, compacting.load(Ordering::Acquire), start.elapsed())
}

fn feed(grid: &Arc<Mutex<Grid>>, text: &str) {
    let mut g = grid.lock();
    let mut parser = vte::Parser::new();
    for b in text.bytes() {
        g.feed_byte(&mut parser, b);
    }
}

fn called(value: &Json, name: &str) -> bool {
    if value.text("type") == "tool_use" && value.text("name") == name {
        return true;
    }
    match value {
        Json::Arr(values) => values.iter().any(|v| called(v, name)),
        Json::Obj(fields) => fields.iter().any(|(_, v)| called(v, name)),
        _ => false,
    }
}

fn check(out: &Outcome, label: &str) {
    if out.error {
        // Never print raw provider responses, request bodies, endpoints or credentials.
        let status = out
            .text
            .strip_prefix("AI HTTP ")
            .and_then(|s| s.split('.').next())
            .and_then(|s| s.parse::<u16>().ok());
        panic!(
            "FAIL {label}; HTTP status: {status:?}; response body suppressed. Verify model access, API credentials and network."
        );
    }
}

#[test]
#[ignore = "live Anthropic requests; use bash scripts/test-ai-live.sh explicitly"]
fn anthropic_live_smoke() {
    assert_eq!(
        std::env::var("LITTY_AI_LIVE_TEST").as_deref(),
        Ok("1"),
        "Use bash scripts/test-ai-live.sh; this test calls the live API."
    );
    let mut settings = config::Settings::default();
    settings.set("provider", "anthropic").unwrap();
    settings
        .set("base-url", "https://api.anthropic.com/v1/messages")
        .unwrap();
    // The runner selects an explicit model so old production defaults cannot
    // silently select a retired snapshot. No persistent config is modified.
    let model = std::env::var("LITTY_AI_LIVE_MODEL").unwrap_or_else(|_| "claude-sonnet-5-5".into());
    settings.set("model", &model).unwrap();
    settings.set("thinking", "off").unwrap();
    settings.set("web-search", "false").unwrap();
    let c = config::Config::load(&settings).unwrap_or_else(|_| {
        panic!("Anthropic credentials unavailable; use the runner to enter the key privately.")
    });
    let grid = Arc::new(Mutex::new(Grid::new(120, 24)));
    feed(
        &grid,
        "Synthetic smoke fixture: LITTY_SCROLLBACK_7789\r\n\x1b]133;A\x07$ \x1b]133;B\x07",
    );
    let timeout = Duration::from_secs(90);

    let (out, _, _) = invoke(
        &c,
        Session::default(),
        "Compatibility test: use the docs tool and the scrollback tool (20 lines), then use the chat tool to return the LITTY_SCROLLBACK marker found in the terminal output. Do not suggest a shell command. Only use these tools; do not answer from memory.",
        &grid,
        timeout,
    );
    check(&out, "docs/scrollback/chat");
    for name in ["docs", "scrollback", "chat"] {
        assert!(
            out.session.history.iter().any(|m| called(m, name)),
            "FAIL required tool was not used: {name}"
        );
    }
    assert!(
        out.text.contains("LITTY_SCROLLBACK_7789"),
        "FAIL scrollback marker missing from chat answer"
    );
    assert!(
        out.command.is_none(),
        "FAIL chat unexpectedly suggested a command"
    );
    println!("PASS live docs + scrollback tool round trips + chat");

    let (mut out, _, _) = invoke(
        &c,
        Session::default(),
        "Two-step compatibility test. First use the command tool to suggest exactly printf LITTY_ORIGINAL_8841 with pending=true. Do not execute it. After I report execution, use chat to state the actual output and stop. The user may edit the command before execution.",
        &grid,
        timeout,
    );
    check(&out, "pending suggestion");
    assert!(
        out.command
            .as_ref()
            .is_some_and(|(cmd, pending)| *pending && cmd.contains("LITTY_ORIGINAL_8841")),
        "FAIL pending command suggestion missing"
    );
    // Never execute model-generated text. Execute only this fixed, harmless fixture.
    let executed = std::process::Command::new("/usr/bin/printf")
        .arg("LITTY_EDITED_8841")
        .output()
        .expect("fixture printf");
    assert!(executed.status.success());
    assert_eq!(executed.stdout, b"LITTY_EDITED_8841");
    out.session.record_execution("printf LITTY_EDITED_8841");
    feed(
        &grid,
        "\r\nLITTY_EDITED_8841\r\n\x1b]133;A\x07$ \x1b]133;B\x07",
    );
    grid.lock().last_exit = Some(0);
    let (out, _, _) = invoke(
        &c,
        out.session,
        "[continuation] Actual command: printf LITTY_EDITED_8841; output: LITTY_EDITED_8841; exit 0. Use chat to state the actual output marker and stop; do not suggest another command.",
        &grid,
        timeout,
    );
    check(&out, "edited-command continuation");
    assert!(
        out.text.contains("LITTY_EDITED_8841") && out.command.is_none(),
        "FAIL continuation did not acknowledge actual edited command output"
    );
    println!("PASS live pending suggestion + actual edited execution + continuation");

    let mut compact_config = c.clone();
    compact_config.context_window = 4096;
    let mut session = Session {
        identity: format!("{}\n{}\n{}", c.provider.name(), c.model, c.url),
        ..Session::default()
    };
    for turn in 0..4 {
        session.history.push(provider::message(
            "user",
            &format!("Synthetic fixture turn {turn}: {}", "padding ".repeat(750)),
        ));
        session.history.push(provider::message(
            "assistant",
            "Noted synthetic test context.",
        ));
        session.boundaries.push(session.history.len());
    }
    let (out, compacted, _) = invoke(
        &compact_config,
        session,
        "Use chat to answer exactly LITTY_COMPACTION_OK. Do not suggest commands.",
        &grid,
        timeout,
    );
    check(&out, "compaction");
    assert!(
        compacted
            && out
                .session
                .history
                .first()
                .is_some_and(|m| m.text("content").starts_with("[context compacted]")),
        "FAIL real summarization did not compact the conversation"
    );
    assert!(
        out.text.contains("LITTY_COMPACTION_OK"),
        "FAIL post-compaction answer missing"
    );
    println!("PASS live context compaction + post-compaction request");

    let (out, _, elapsed) = invoke(
        &c,
        Session::default(),
        "Use chat to write a detailed 2000-word essay about terminal emulators. This request will be cancelled by a client transport test.",
        &grid,
        Duration::from_millis(200),
    );
    assert!(
        out.error && out.text == "Cancelled",
        "FAIL live transport cancellation"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "FAIL cancellation exceeded two seconds"
    );
    println!(
        "PASS live transport cancellation in {} ms",
        elapsed.as_millis()
    );
    println!(
        "PASS Anthropic live smoke suite; no persistent config changed and no model-generated command executed"
    );
}
