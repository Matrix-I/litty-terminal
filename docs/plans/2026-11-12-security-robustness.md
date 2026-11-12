# Security and robustness Implementation Plan

> **For agentic workers:** Use subagent-driven-development with code-review-strict. Steps use `- [ ]`. No commits without a user request.

**Goal:** Resolve the eleven findings in the user's screenshot, after reproduction on main at `6874d12d22c84e9b655fb47ad7a0df61c7a23335` (macOS).
**Architecture:** Keep byte transformations pure; keep filesystem and process work off the UI thread and outside grid locks. Stage complete macOS bundles before an atomic exchange. Bind the existing signed checksum manifest to the requested release tag.
**Tech Stack:** Rust 2024, existing libc and standard library; no new dependencies.
**Branch:** `fix/security-robustness`.

## Confirmed Requirements

User approved all three choices on 2026-11-12:
- Add a release version to the signed manifest; reject old manifests without it.
- Bound copy/history queues; report overload and preserve a history entry when its deletion cannot be accepted.
- Reject unsupported Kitty delete selectors without changing stored images.

The remaining screenshot requirements: atomic macOS bundle installation; paste LF to CR conversion; no synchronous two-second reap in reader setup failure; no file-existence leak through failed Kitty decodes; assigned id in image-number replies; valid APNG subframes must not prevent static default-image decoding; background history reload; stable AI system prefix and no false skipped-command record; correct paste documentation.

## Global Constraints

`AGENTS.md` and `CLAUDE.md` apply in full: 12-byte cells; no blocking I/O under grid locks; no hot-path allocations; platform parity; async-signal-safe post-fork operations; sanitized paste; raw-byte atomic history writes. No public/config/dependency changes beyond the approved signed manifest change. No commit/push/release. Read files and callers before edits. Reproduce each bug and capture a failing regression before implementing; report unreproduced findings rather than speculative fixes.

Required final gate, after the last edit: read full diff and untracked files; `cargo check`; `cargo check --target x86_64-unknown-linux-gnu`; `cargo test` outside sandbox; `cargo clippy --all-targets -- -D warnings`; `git diff --check`; nine-point self-review and independent code review. Run real flows where safe; report environment gaps explicitly.

## Technical Defaults

- Keep manifest name SHA256SUMS.txt and append a signed comment containing the exact release tag so checksum tools remain compatible.
- Paste converts CRLF to one CR and lone LF to CR, preserving lone CR and existing ESC/C1 filtering.
- APNG support here is static fallback, not animation playback. Validate malformed default-image data independently.
- Use existing event wakeups for background history completion, with generation checks to prevent stale results undoing edits.

## Risks & Rollback

- Version binding deliberately rejects older manifests; updater and signing workflow must ship together. Revert both together to roll back.
- Atomic bundle replacement needs the same filesystem and complete signing before activation; keep the installed bundle intact on any staging failure.
- Background history results can race with session additions/deletions: merge current session state and discard stale generations.
- Bounded queue overload must be observable without logging copied commands or clipboard content.
- Linux GUI and a real release install may be unavailable locally; record alternatives and gaps.

## Task 1: Paste and AI lifecycle

Files: `src/paste.rs`, `src/ai/mod.rs` (including their existing tests).
Interfaces: `paste_payload(&str, bool) -> Vec<u8>`; `PaneState::cancel_request(&mut self)`; `run` builds provider messages.
- [x] Add failing observable payload tests for LF, CRLF, lone CR, Unicode, and bracketed escape filtering; cancellation tests with and without a suggestion; provider request test showing last-exit updates do not mutate the system prefix.
- [x] Run focused tests and record exact baseline failures.
- [x] Normalize newlines, move paste rustdoc, record skipped only for a pending suggestion, and move last-exit context into the changing user message.
- [x] Run subsystem suites and review diff. Ready to commit (only when asked): `fix(paste): normalize newlines and correct ai cancellation context`.

## Task 2: Kitty and APNG

Files: `src/terminal/graphics/{kitty.rs,png.rs,tests.rs}`.
Interfaces: `Store::finish_kitty`, `decode_kitty`, `decode_png` retain their existing signatures.
- [x] Reproduce unsupported delete clearing, image-number i=0 reply, file decode error discrepancy, and a valid APNG with a full default image followed by a smaller animation frame.
- [x] Add regression tests, confirm failures, and record output.
- [x] Reject unsupported selectors before mutation; return assigned image id; normalize file decode failures; allow valid later APNG frames while retaining default-image bounds validation.
- [x] Run graphics suites and review. Ready to commit: `fix(graphics): preserve images and harden protocol responses`.

## Task 3: Update authenticity and atomic installation

Files: `src/update/{install.rs,verify.rs}`, `.github/workflows/release.yml`, updater tests.
Interfaces: `download_and_verify_in` and `apply_update` keep their signatures; new internal helpers may receive injected filesystem/process boundaries for deterministic tests.
- [x] Test manifest missing/mismatched/duplicate version, exact matching version, cached and fresh downloads; reproduce directory overlay leaving old contents.
- [x] Confirm failures; add version validation on both verified download paths and signed tag comment in release workflow.
- [x] Stage and sign a complete sibling bundle and atomically exchange directories on macOS; test success/failure preserving the installed bundle with temporary fixture bundles. Never install over the user's actual app.
- [x] Run update suites and review. Ready to commit: `fix(update): bind manifests to tags and exchange staged bundles`.

## Task 4: Bounded asynchronous history and clipboard; PTY failure

Files: `src/platform/clipboard.rs`, `src/terminal/{history.rs,pty.rs}`, `src/app/history_ui.rs`, and only necessary event-loop integration callers.
Interfaces: preserve public platform API; internal history APIs may expose acceptance/error state to the popup. Follow existing event proxy/waker patterns.
- [x] Reproduce saturation drops and reader-setup synchronous reap through injected true boundaries, not timing/sleep assertions; prove history filesystem work executes on a worker.
- [x] Capture failures, then implement bounded acceptance with explicit diagnostics, preserve rejected deletions, background reload with stale-result protection, and leave reaping ownership with the PTY on failed reader setup.
- [x] Exercise acceptance, full/disconnected queues, spawn/I/O failures and stale reload completion in deterministic tests; run full affected suites.
- [x] Review. Ready to commit: `fix(history): bound background work without silent request loss`.

## Task 5: Verification and documentation

- [x] Update README user-visible behavior, update guide manifest compatibility, and ROADMAP completed findings only after verification.
- [x] Run the complete final gate above and record exact results in this plan.
- [x] Independently review full diff and requirements; address findings and repeat final gate after edits.
- [x] Report nine checklist items with concrete reasons and all incomplete/unverified checks, including real GUI/release gaps. Do not commit.

## Evidence and deviations

Implementation briefs must include concrete regression test and minimal implementation code before changes. This plan records acceptance criteria and subsystem boundaries; detailed code is prepared against the actual test harness in each task rather than guessing test fixtures here. This is a deviation from writing-plans-strict's up-front full-code template, subject to plan review.

### Plan review (2026-11-12)
Reviewer approved architecture after tightening Task 4: serialize accepted history deletions and reloads on one worker, or maintain deletion tombstones until persistence completes; generation alone is insufficient. Move discovery/stat/existence checks off UI as well as parsing. Test dup and thread-spawn failure retain exactly one reaper. Keep invalid default APNG rejection and add valid later subframe fixture.
Model deviation: the skill requests opus, which is unavailable in this environment; review uses the session model.

### Task 3 baseline
`cargo test manifest_is_bound_to_exact_release_tag -- --nocapture`: FAILED, baseline accepts missing version comment. Regression uses existing `pick_asset`, real checksum parsing, and exact requested tag.

### Completed subsystem evidence

- Task 1: paste 8 pass; AI 36 pass, 1 existing credential-gated ignored test. Failing pre-fix payload, false-skipped and differing provider system-message tests recorded in `/tmp/litty-security-task-1-report.md`. Independent spec/quality review clean.
- Task 2: four baseline protocol regressions failed; graphics 39 pass, 1 existing ignored benchmark. Static fallback uses valid full default IDAT followed by smaller frame. Independent review clean; corrected decoder comments. Evidence `/tmp/litty-security-task-2-report.md`.
- Task 3: baseline missing-version regression failed; baseline ditto fixture retained obsolete file. Updater 24 pass after changes, including exact/missing/mismatched/duplicate version through fresh/cache paths (only download/signature subprocess boundaries injected), real ZIP extraction/signing/executable invocation/signature verification, and injected signing/touch/activation failures preserving old app. Atomic exchange fixture passes. Two review rounds, all three findings fixed. Base installer has no injectable destination and refuses dev binaries, so the baseline bundle reproduction used the same real ditto operation on disposable directories; new helpers allow automated install-flow coverage.
- Task 4 PTY: baseline injected duplication failure killed/reaped an owned live child and failed after 2.17 s. Full PTY suite 15 pass after changes. Review found immediate caller-drop still waited; fixed by terminating failed-start child after descriptors close and handing waiting to a shared bounded reaper. Regression observes actual reaping on a different thread for duplication and thread-spawn failures, then ECHILD. Queue full/disconnected tests preserve cleanup ownership. Second review approved. Under reaper resource exhaustion, fallback synchronously waits an already-SIGKILLed child without the two-second grace; this prevents zombies.
- Packaging: `bash bundle_macos.sh --no-install` failed inside sandbox at hdiutil (`Device not configured`), then succeeded outside sandbox. `minisign -Vm dist/SHA256SUMS.txt -p resources/minisign.pub` verified the generated signature and marker `# litty-release: v0.11.1`. No installed app was replaced or release published.
- GUI verification: attempted Computer Use to inspect Litty; unavailable because Computer Use permissions are not granted. Real bundle fixture, subprocess and PTY flows are verified instead; popup visuals and interactive clipboard UI remain unverified.

### Workflow deviations

The strict plan template's complete up-front implementation/test code was deferred to subsystem briefs; root tasks 3/PTY used concrete repro/test changes rather than full prewritten code. This is an explicit process deviation, not a change in accepted behavior. The required `opus` review model is unavailable; independent reviews used the available session model. No commit or push is authorized.
