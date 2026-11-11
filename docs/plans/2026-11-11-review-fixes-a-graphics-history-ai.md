# Review fixes A (graphics, history, AI) Implementation Plan — 2026-11-11

> **For agentic workers:** REQUIRED SUB-SKILL: development-superpowers:subagent-driven-development or executing-plans-strict. Steps use `- [ ]`.

**Goal:** Fix findings 2, 6, 7 and 10 of
[the 2026-11-11 review](../reviews/2026-11-11-linux-history-ai-graphics.md)
and record the review's secondary findings in `ROADMAP.md`. Findings 3, 4, 5, 8
are in [plan B](./2026-11-11-review-fixes-b-vulkan-robustness.md); findings 1
and 9 are in [plan C](./2026-11-11-review-fixes-c-vulkan-images.md).

**Architecture:** Four independent changes, one commit each, all in
platform-neutral code that runs on macOS:

1. Kitty animation playback does not overflow on huge frame gaps and loop counts (`src/terminal/graphics/animation.rs`).
2. Fish history is unescaped in one left-to-right pass (`src/terminal/history.rs`).
3. Deleting a history entry removes every occurrence from the file (`src/terminal/history.rs`).
4. The AI transport retries only failures another attempt can fix (`src/ai/http.rs`).

**Tech Stack:** Rust 2024, no new dependency.

**Branch:** `fix/review-graphics-history-ai` (from `main`).

## Confirmed Requirements

1. Deleting a command in the history popup removes **every** occurrence of it from the shell history file, each with a directly preceding bash timestamp comment. The test `removal_only_deletes_single_latest_occurrence` is rewritten for this behaviour. (user, 2026-11-11)
2. The ten main findings are fixed in three independent plans, one branch each; this is plan A. (user, 2026-11-11)
3. Secondary findings are not fixed here; they become `ROADMAP.md` follow-ups. (user request for this plan)

## Global Constraints

From `CLAUDE.md` (project):

- Performance: zero heap allocation on hot paths (PTY parsing, scrolling, render loop). None of the four changes is on such a path.
- Shell history writes: mutate raw bytes, write temp file, atomic `rename`. Task 3 keeps `remove_command_from_file` unchanged and only changes which entries are dropped.
- Bug fixes start with a failing regression test that reproduces the bug. `grid` aside, unit tests live in the same file (`graphics` keeps them in `src/terminal/graphics/tests.rs`).
- Scope: change only what the task needs.
- Verification before every commit that touches `src/`:
  - `cargo check`
  - `cargo check --target x86_64-unknown-linux-gnu`
  - `cargo test` (outside the sandbox; PTY tests require `openpty`)
  - `cargo clippy --all-targets -- -D warnings`
  - `git diff --check`
- Commit only when asked; never push. Conventional Commits `type(scope): summary`, lowercase, imperative, no trailing period, at most 72 characters; body explains why. Stage files explicitly. No `Co-Authored-By` trailer.
- Update `ROADMAP.md` when a roadmap item is completed or descoped; update `README.md` when user-visible behaviour changes.

From the user's global rules: tests must be able to fail and be deterministic
(no sleeps, no wall-clock dependence); report results honestly; self-review
checklist and independent review before reporting done.

## Technical Defaults

- Finding 2 as written (a panic from `Instant - Duration`) was **not reproducible**: on macOS (rustc 1.99.1) `Instant::now() - 13.6 years` succeeds, because the Unix `Instant` may go below its origin. No change is made for it (no speculative fix). What does reproduce is an overflow of `cycle * plays` in `Image::frame_at`: a panic in debug builds, a wrapped product (wrong frame) in release builds, which have no overflow checks. Task 1 fixes that.
- `cycle * plays` uses `saturating_mul` — same result for every representable product.
- Only curl exit codes that describe the network are retried: 5 and 6 (name resolution), 7 (connect), 16 (HTTP/2 framing), 18 (partial transfer), 28 (timeout), 35 (TLS handshake), 52 (empty reply), 55 and 56 (send / receive), 92 (HTTP/2 stream not closed cleanly). 16 and 92 are included because LLM APIs are reached over HTTPS with HTTP/2, where a reset stream is transient. Every other failure is fatal at once: any other curl exit (malformed URL, `--proto`, `--max-filesize`, killed by a signal), curl cannot be started or waited for, pipe errors, a response over 8 MiB, a non-UTF-8 body, a missing or invalid status line. HTTP status handling (408, 429, 5xx retried) is unchanged.
- Behaviour change: curl exiting 0 while writing its stdin config failed (`written.is_err()`) was retried before and is now fatal (`transient_curl_exit(Some(0))` is false). curl read its whole config before a successful exit, so this cannot be a network failure; it is rare and not worth a retry.
- An unknown fish escape (`\x`) is kept verbatim as backslash + character, as the current code does.

## Risks & Rollback

| Risk | Mitigation | Revert |
|---|---|---|
| Task 3 deletes more lines from the user's history file than before | Confirmed requirement 1; the atomic temp-file + `rename` path and the "file changed meanwhile" retry are untouched; tests cover timestamps and non-matching entries | Revert the Task 3 commit; no file format changes |
| Task 4 turns a transient failure into an immediate error | The retried curl exit codes are listed under Technical Defaults and pinned by a unit test; a missed transient code costs one manual retry | Revert the Task 4 commit |
| Task 1 changes when a finite animation stops | Only for products above `u64::MAX` ms, which previously wrapped to an arbitrary value | Revert the Task 1 commit |

No persisted data format, public API, config key or keybinding changes.

---

## Task 1: Animation loop arithmetic cannot overflow (finding 2, corrected)

The panic the review described (`Instant - Duration`) does not reproduce on
macOS; see Technical Defaults. The reproducible defect in the same code path
is `cycle * plays` in `Image::frame_at`.

**Files:**
- Test: `src/terminal/graphics/tests.rs` (append after `kitty_animation_frames_play_from_the_clock`, which ends near line 96)
- Modify: `src/terminal/graphics/animation.rs:98`

**Interfaces:** Consumes `Store::kitty_animate(&mut self, cmd: &KittyCommand, now: Instant) -> Result<(), String>`, `Image::frame_at(&self, now: Instant) -> (usize, Option<Instant>)` and the test helper `run(store: &mut Store, cmd: &str) -> Option<String>`. Produces no new symbol.

- [x] **Step 1: Write the failing test**

Append to `src/terminal/graphics/tests.rs`:

```rust
#[test]
fn kitty_animation_loop_length_does_not_overflow() {
    let mut store = Store::default();
    run(&mut store, "Ga=t,f=24,s=1,v=1,i=7,q=2;/wAA");
    // 200 frames of i32::MAX ms each and the largest loop count: the total
    // play time (cycle * plays) is far above u64::MAX milliseconds.
    for _ in 0..200 {
        run(&mut store, "Ga=f,f=24,i=7,z=2147483647,q=2;AP8A");
    }
    let now = Instant::now();
    let (cmd, _) = parse_kitty(b"Ga=a,i=7,s=3,c=1,v=4294967295").unwrap();
    store.kitty_animate(&cmd, now).unwrap();
    let (frame, next) = store.image(7).unwrap().frame_at(now);
    assert_eq!(frame, 0, "still inside the first frame of the first play");
    assert_eq!(next, Some(now + Duration::from_millis(40)), "root frame keeps its 40 ms gap");
}
```

- [x] **Step 2: Run it and confirm the failure**

Run: `cargo test --lib kitty_animation_loop_length_does_not_overflow`
Expected: FAIL with a panic `attempt to multiply with overflow` at `animation.rs:98` (test builds have overflow checks).

- [x] **Step 3: Fix**

In `src/terminal/graphics/animation.rs`, inside `Image::frame_at`, replace

```rust
        if plays != 0 && elapsed >= cycle * plays as u64 {
```

with

```rust
        // Gaps and the loop count come from program output: their product can
        // exceed u64, which must read as "not finished yet", not wrap around.
        if plays != 0 && elapsed >= cycle.saturating_mul(plays as u64) {
```

- [x] **Step 4: Run the tests**

Run: `cargo test --lib graphics`
Expected: PASS, including `kitty_animation_frames_play_from_the_clock`.

- [ ] **Step 5: Commit checkpoint**

Ready to commit: `src/terminal/graphics/animation.rs`, `src/terminal/graphics/tests.rs` — `fix(graphics): saturate the animation loop length`

---

## Task 2: Fish history unescape in one pass (finding 6)

**Files:**
- Modify: `src/terminal/history.rs:341-363` (`split_fish_entries`)
- Test: `src/terminal/history.rs` tests module (after `removal_drops_fish_block`, near line 1211)

**Interfaces:** Produces `fn unescape_fish(s: &str) -> String` (private). Consumes the test helpers `temp_history(name: &str, content: &[u8]) -> PathBuf`, `parse_history_file(path: &Path, out: &mut Vec<String>)` and `remove_command_from_file(path: &Path, cmd: &str) -> std::io::Result<bool>`.

- [x] **Step 1: Write the failing test**

Add to the tests module of `src/terminal/history.rs`:

```rust
    #[test]
    fn fish_escaped_backslash_before_n_stays_a_backslash() {
        // On disk: `printf 'a\\nb'` (escaped backslash, then `n`) and `echo x\ny` (a newline).
        let content = b"- cmd: printf 'a\\\\nb'\n  when: 1\n- cmd: echo x\\ny\n  when: 2\n";
        let path = temp_history("fish_history", content);
        let mut out = Vec::new();
        parse_history_file(&path, &mut out);
        assert_eq!(out, vec!["echo x\ny", "printf 'a\\nb'"]);
        assert!(remove_command_from_file(&path, "printf 'a\\nb'").expect("remove"));
        assert_eq!(std::fs::read(&path).unwrap(), b"- cmd: echo x\\ny\n  when: 2\n");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
```

- [x] **Step 2: Run it and confirm the failure**

Run: `cargo test --lib fish_escaped_backslash_before_n_stays_a_backslash`
Expected: FAIL on the first `assert_eq!`: the second element is `"printf 'a\\\nb'"` (backslash, real newline) instead of `"printf 'a\\nb'"`.

- [x] **Step 3: Fix**

In `src/terminal/history.rs`, add above `split_fish_entries`:

```rust
/// Reverse fish's history escaping in one pass: `\\` is a backslash and `\n`
/// a newline. Two `replace` calls would read the `\n` inside `\\n` first.
fn unescape_fish(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}
```

and in `split_fish_entries` replace

```rust
            let cmd = rest.trim().replace("\\n", "\n").replace("\\\\", "\\");
```

with

```rust
            let cmd = unescape_fish(rest.trim());
```

- [x] **Step 4: Run the tests**

Run: `cargo test --lib history`
Expected: PASS, including `removal_drops_fish_block`.

- [ ] **Step 5: Commit checkpoint**

Ready to commit: `src/terminal/history.rs` — `fix(history): unescape fish history in a single pass`

---

## Task 3: Deleting a history entry removes every occurrence (finding 7)

**Files:**
- Modify: `src/terminal/history.rs:480-502` (`strip_command_entries`)
- Test: `src/terminal/history.rs`, replace `removal_only_deletes_single_latest_occurrence` (near line 1238)

**Interfaces:** `fn strip_command_entries(path: &Path, bytes: &[u8], cmd: &str) -> Option<Vec<u8>>` keeps its signature; `None` still means "nothing matched".

- [x] **Step 1: Replace the old test with the failing one**

Delete the test `removal_only_deletes_single_latest_occurrence` (it asserts the behaviour confirmed requirement 1 replaces) and add:

```rust
    #[test]
    fn removal_deletes_every_occurrence_with_its_timestamp() {
        let content = b"#1\necho repeated\necho first\n#2\necho repeated\necho middle\necho repeated\n";
        let path = temp_history(".bash_history", content);
        assert!(remove_command_from_file(&path, "echo repeated").expect("remove"));
        assert_eq!(std::fs::read(&path).unwrap(), b"echo first\necho middle\n");
        assert!(!remove_command_from_file(&path, "echo repeated").expect("no-op"), "nothing left to remove");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
```

- [x] **Step 2: Run it and confirm the failure**

Run: `cargo test --lib removal_deletes_every_occurrence_with_its_timestamp`
Expected: FAIL on the first `assert_eq!`: the file still holds `#1\necho repeated\necho first\n#2\necho repeated\necho middle\n`.

- [x] **Step 3: Fix**

Replace the whole `strip_command_entries` function (doc comment included) with:

```rust
/// Return `bytes` with every entry equal to `cmd` removed (each along with a bash
/// timestamp comment directly preceding it), or `None` if nothing matched.
/// The popup lists a command once, so deleting it must leave no copy behind.
/// All other entries are kept byte-for-byte, so metafied zsh data is never re-encoded.
fn strip_command_entries(path: &Path, bytes: &[u8], cmd: &str) -> Option<Vec<u8>> {
    let entries = split_history_entries(path, bytes);
    let mut kept: Vec<&RawEntry> = Vec::with_capacity(entries.len());
    let mut removed = false;
    for entry in &entries {
        if entry.command.as_deref() == Some(cmd) {
            if kept.last().is_some_and(|e| e.is_timestamp) {
                kept.pop();
            }
            removed = true;
            continue;
        }
        kept.push(entry);
    }
    if !removed {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len());
    for entry in kept {
        out.extend_from_slice(&bytes[entry.span.clone()]);
    }
    Some(out)
}
```

- [x] **Step 4: Run the tests**

Run: `cargo test --lib history`
Expected: PASS, including `removal_drops_bash_timestamp_and_keeps_permissions`, `zsh_metafied_entries_decode_and_survive_removal`, `removal_drops_whole_multiline_entry` and `removal_drops_fish_block`.

- [ ] **Step 5: Commit checkpoint**

Ready to commit: `src/terminal/history.rs` — `fix(history): delete every occurrence of a removed command`

---

## Task 4: Retry only transient AI transport failures (finding 10)

**Files:**
- Modify: `src/ai/http.rs:17-175` (`once`, `request`) and the existing `cancellation_tests`
- Test: `src/ai/http.rs`, new module `retry_tests` at the end of the file

**Interfaces:**
- Produces `struct Failure { message: String, retry: bool }`, `fn fatal(message: &str) -> Failure` and `fn transient_curl_exit(code: Option<i32>) -> bool` (all private).
- `once` becomes `fn once(url: &str, headers: &[String], body: Option<&str>, cancel: &Arc<AtomicBool>) -> Result<Reply, Failure>`.
- `request` keeps `pub fn request(url: &str, headers: &[String], body: Option<&str>, cancel: &Arc<AtomicBool>, status: &impl Fn(String), last: &mut String) -> Result<String, String>`.

- [x] **Step 1: Write the failing test**

Append to `src/ai/http.rs`:

```rust
#[cfg(test)]
mod retry_tests {
    use super::*;

    #[test]
    fn permanent_transport_errors_are_not_retried() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            // Non-blocking accept with a deadline: if curl never connects the
            // test fails instead of hanging.
            listener.set_nonblocking(true).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                if let Ok((s, _)) = listener.accept() {
                    break s;
                }
                assert!(Instant::now() < deadline, "curl never connected");
                std::thread::sleep(Duration::from_millis(5));
            };
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut raw = Vec::new();
            let mut buf = [0u8; 1024];
            while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&buf[..n]);
            }
            // A body that is not UTF-8: no later attempt can change that.
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\n\xff")
                .unwrap();
        });
        let cancel = Arc::new(AtomicBool::new(false));
        let statuses = std::sync::Mutex::new(Vec::new());
        let mut last = String::new();
        let result = request(
            &url,
            &[],
            None,
            &cancel,
            &|text| {
                statuses.lock().unwrap().push(text);
                // A retry would back off for seconds; cancel so unfixed code fails fast.
                cancel.store(true, Ordering::Release);
            },
            &mut last,
        );
        server.join().unwrap();
        assert_eq!(result.err().as_deref(), Some("AI response is not UTF-8"));
        assert!(statuses.lock().unwrap().is_empty(), "a retry was announced: {:?}", statuses.lock().unwrap());
    }
}
```

- [x] **Step 2: Run it and confirm the failure**

Run: `cargo test --lib permanent_transport_errors_are_not_retried`
Expected: FAIL with `left: Some("Cancelled")`: the unfixed `request` announces attempt 2, the callback cancels, and the backoff returns `Cancelled`.

- [x] **Step 3: Fix**

In `src/ai/http.rs`, add below the `Reply` struct:

```rust
/// A failed attempt. `retry` is set only for failures another attempt can fix
/// (the network), never for ones that repeat identically (no curl, bad reply).
#[derive(Debug)]
struct Failure {
    message: String,
    retry: bool,
}
fn fatal(message: &str) -> Failure {
    Failure { message: message.into(), retry: false }
}
/// curl exit codes that describe the network at this moment: name resolution
/// (5, 6), connect (7), HTTP/2 framing (16), partial transfer (18), timeout
/// (28), TLS handshake (35), empty reply (52), send / receive (55, 56), HTTP/2
/// stream reset (92). Anything else (bad URL, `--proto`, `--max-filesize`, a
/// signal) fails the same way on every attempt.
fn transient_curl_exit(code: Option<i32>) -> bool {
    matches!(code, Some(5 | 6 | 7 | 16 | 18 | 28 | 35 | 52 | 55 | 56 | 92))
}
```

Replace the whole `once` function with:

```rust
fn once(
    url: &str,
    headers: &[String],
    body: Option<&str>,
    cancel: &Arc<AtomicBool>,
) -> Result<Reply, Failure> {
    cancelled(cancel).map_err(|e| fatal(&e))?;
    let mut config = format!("url = {}\n", quote(url));
    for h in headers {
        config.push_str(&format!("header = {}\n", quote(h)));
    }
    if let Some(b) = body {
        config.push_str("request = \"POST\"\n");
        // data-raw: data-binary treats a leading '@' as a local file to upload (CWE-73).
        config.push_str(&format!("data-raw = {}\n", quote(b)));
    }
    let bin = if std::path::Path::new("/usr/bin/curl").exists() {
        "/usr/bin/curl"
    } else {
        "curl"
    };
    // No redirects: credentials must never follow a redirect to another origin.
    let mut child = Command::new(bin)
        .args([
            "--disable",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "10",
            "--max-time",
            "180",
            "--max-filesize",
            "8388608",
            "--proto",
            "=http,https",
            "--write-out",
            "\n%{http_code}",
            "--config",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| fatal("Cannot start curl"))?;
    let mut stdin = child.stdin.take().ok_or_else(|| fatal("curl stdin unavailable"))?;
    let stdout = child.stdout.take().ok_or_else(|| fatal("curl stdout unavailable"))?;
    let stderr = child.stderr.take().ok_or_else(|| fatal("curl stderr unavailable"))?;
    // Drain both pipes concurrently: large requests/responses must not deadlock.
    let writer = std::thread::spawn(move || stdin.write_all(config.as_bytes()));
    let reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        stdout
            .take(MAX_RESPONSE + 16)
            .read_to_end(&mut b)
            .map(|_| b)
    });
    let errors = std::thread::spawn(move || {
        let mut b = Vec::new();
        stderr.take(64 * 1024).read_to_end(&mut b).map(|_| b)
    });
    let status = loop {
        if cancel.load(Ordering::Acquire) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer.join();
            let _ = reader.join();
            let _ = errors.join();
            return Err(fatal("Cancelled"));
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
                let _ = reader.join();
                let _ = errors.join();
                return Err(fatal("Cannot wait for curl"));
            }
        }
    };
    let written = writer.join().map_err(|_| fatal("curl writer failed"))?;
    let bytes = reader
        .join()
        .map_err(|_| fatal("curl reader failed"))?
        .map_err(|_| fatal("Cannot read curl response"))?;
    let _ = errors.join();
    cancelled(cancel).map_err(|e| fatal(&e))?;
    if !status.success() || written.is_err() {
        return Err(Failure {
            message: format!("AI transport failed (curl exit {:?})", status.code()),
            retry: transient_curl_exit(status.code()),
        });
    }
    if bytes.len() as u64 > MAX_RESPONSE {
        return Err(fatal("AI response exceeds 8 MiB limit"));
    }
    let text = String::from_utf8(bytes).map_err(|_| fatal("AI response is not UTF-8"))?;
    let (body, code) = text.rsplit_once('\n').ok_or_else(|| fatal("Missing HTTP status"))?;
    Ok(Reply {
        status: code.parse().map_err(|_| fatal("Invalid HTTP status"))?,
        body: body.into(),
    })
}
```

In `request`, replace

```rust
            Err(e) if e == "Cancelled" => return Err(e),
            Err(e) => e,
```

with

```rust
            Err(failure) if !failure.retry => return Err(failure.message),
            Err(failure) => failure.message,
```

In `cancellation_tests::cancels_an_inflight_curl_process`, replace

```rust
        assert_eq!(result.err().as_deref(), Some("Cancelled"));
```

with

```rust
        assert_eq!(result.err().map(|f| f.message).as_deref(), Some("Cancelled"));
```

Then add this test to `retry_tests`, to pin the retried exit codes:

```rust
    #[test]
    fn only_network_curl_exits_are_transient() {
        for code in [5, 6, 7, 16, 18, 28, 35, 52, 55, 56, 92] {
            assert!(transient_curl_exit(Some(code)), "{code}");
        }
        // Success with a failed stdin write, unsupported protocol, malformed
        // URL, write error, --max-filesize, killed by a signal.
        for code in [Some(0), Some(1), Some(3), Some(23), Some(63), None] {
            assert!(!transient_curl_exit(code), "{code:?}");
        }
    }
```

- [x] **Step 4: Run the tests**

Run: `cargo test --lib ai::`
Expected: PASS, including `permanent_transport_errors_are_not_retried`, `only_network_curl_exits_are_transient`, `cancels_an_inflight_curl_process`, both `body_tests`, `retry_policy_and_cancelled_backoff` and the three `integration_tests` in `src/ai/mod.rs`.

- [ ] **Step 5: Commit checkpoint**

Ready to commit: `src/ai/http.rs` — `fix(ai): stop retrying transport failures that cannot succeed`

---

## Task 5: Verification & Docs

**Files:**
- Modify: `ROADMAP.md` (append a section at the end of the file)
- Modify: `docs/reviews/2026-11-11-linux-history-ai-graphics.md` (status notes)

- [x] **Step 1: Record the secondary findings in `ROADMAP.md`**

Append at the end of `ROADMAP.md`:

```markdown
## Review follow-ups (2026-11-11)

From [the 2026-11-11 review](./docs/reviews/2026-11-11-linux-history-ai-graphics.md). Not scheduled.

- [ ] **Linux: Wayland window backend.** `NativeWindow::create` is X11 only; the Wayland surface and clipboard paths are unreachable.
- [ ] **Linux: IME and dead keys.** `set_ime_allowed` is a no-op and X11 text comes from `XLookupString` without XIM.
- [ ] **Linux: `_NET_WM_NAME` and `WM_CLASS`.** Non-ASCII titles are garbled and desktops cannot match the window to its `.desktop` entry.
- [ ] **Linux: per-window Vulkan frame state.** One command buffer, two descriptor sets and the uniform buffers are shared by all windows.
- [ ] **Linux: check `vkQueueSubmit` and creation results**; pick the physical device by present support and device type.
- [ ] **Linux: custom shaders** (GLSL to SPIR-V, offscreen targets).
- [ ] **Linux: batch glyph uploads** into one submission per frame.
- [ ] **Graphics: Kitty `a=d` selectors** other than `i`/`I` clear everything; confirm the alternate-screen behaviour against the grid caller.
- [ ] **Graphics: `t=f` replies** still reveal whether a file exists (decode errors differ from `EPERM`).
- [ ] **Graphics: `I=` replies** carry `i=0` instead of the assigned id; APNG files with partial frames are rejected.
- [ ] **History: reload on the UI thread** when a history file changed; deletes dropped silently when four are in flight.
- [ ] **Linux: window stubs.** `current_monitor` reports 1920x1080 at 60 Hz, `theme` is always dark, cursor icon, cursor visibility and fullscreen only store a flag.
- [ ] **AI: `last_exit` in the system prompt** invalidates the cached prefix; `cancel_request` records a skipped command when none was suggested.
```

- [x] **Step 2: Mark the fixed findings in the review**

In `docs/reviews/2026-11-11-linux-history-ai-graphics.md`, add this line directly under each of the headings `### 2.` (already corrected to describe the `cycle * plays` overflow), `### 6.`, `### 7.` and `### 10.`:

```markdown
**Status:** fixed by [plan A](../plans/2026-11-11-review-fixes-a-graphics-history-ai.md).
```

and replace the paragraph starting with `- **Open question:**` under `### 7.` with:

```markdown
- **Decision (user, 2026-11-11):** every occurrence is removed.
```

`README.md` needs no change: it does not describe how many occurrences a history delete removes, the AI retry policy or animation limits (checked with `grep -n -i "delete" README.md docs/guide/*.md`).

- [x] **Step 3: Read the full diff**

Run: `git status --short` and `git diff`
Expected: only `src/terminal/graphics/animation.rs`, `src/terminal/graphics/tests.rs`, `src/terminal/history.rs`, `src/ai/http.rs`, `ROADMAP.md` and the review file; no debug output, leftover TODOs or commented-out code.

- [x] **Step 4: Full verification**

Run each and quote the result:

```bash
cargo check
cargo check --target x86_64-unknown-linux-gnu
cargo test
cargo clippy --all-targets -- -D warnings
git diff --check
```

Expected: all succeed, zero warnings. `cargo test` must run outside the sandbox (PTY tests need `openpty`).

- [ ] **Step 5: End-to-end check** _(2026-11-12: not run; checks 1–3 need the GUI. Covered by unit tests only.)_

Run the app (`cargo run`) and, in a pane:

1. In a debug build (`cargo run`, overflow checks on), print the escape sequences of Task 1's test with `printf` (the `a=t`, 200 `a=f` frames with `z=2147483647`, `a=a,i=7,s=3,c=1,v=4294967295`, then place the image with `a=p,i=7`). Expected: the terminal keeps running and shows the first frame.
2. In a shell with a scratch history file (`HISTFILE=$(mktemp) zsh -f`), run `echo dup-074` three times, open the history popup (`Ctrl+R`), delete `echo dup-074`, close and reopen the popup. Expected: the command is gone and `grep -c dup-074 "$HISTFILE"` prints `0`.
3. With `ai-provider = ollama` and `ai-base-url` pointing at a closed loopback port (`http://127.0.0.1:9/v1/chat/completions`), run a `yo` query. Expected: attempts are announced (connection refused is curl exit 7, retryable) and `Ctrl+C` cancels.

Record what was observed. If the GUI cannot be driven in the execution environment, say so and report the three checks as not run.

- [x] **Step 6: Self-review and independent review**

Go through the nine-item self-review checklist of the user's global rules with a one-line reason per item, then request an independent review of the diff (`/code-review` or a review subagent given the diff and this plan's Confirmed Requirements). Address or report its findings; if anything changed, repeat from Step 3.

- [ ] **Step 7: Commit checkpoint**

Ready to commit: `ROADMAP.md`, `docs/reviews/2026-11-11-linux-history-ai-graphics.md` — `docs: record review follow-ups and fixed findings`

**Verification gaps to report:** fish and bash deletion are covered by unit tests only (no real fish shell run); the AI check needs a configured provider key for anything beyond the connection-refused path.
