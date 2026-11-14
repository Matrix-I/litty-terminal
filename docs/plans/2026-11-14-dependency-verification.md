# Dependency Verification Implementation Plan
> **For agentic workers:** REQUIRED SUB-SKILL: executing-plans-strict.

**Goal:** Restore passing tests and dependency auditing.
**Architecture:** Enrich the result of either platform's shader validator with the existing user hint.
**Tech Stack:** Rust, Cargo, Metal, Vulkan/naga, cargo-audit.
**Branch:** `fix/dependency-verification`; preserve the user's existing upgrade changes.

## Confirmed Requirements
- User: "cargo test/audit fails after upgrading the deps version, let's fix them".
- Preserve existing regression expectations; no project dependency or API changes needed.

## Global Constraints
- Follow `CLAUDE.md` and the supplied AGENTS.md; English code and docs.
- No hot-path allocations, grid layout changes, blocking I/O under the grid mutex, or platform divergence.
- Test first; never weaken existing tests. Read the full diff, including untracked files.
- Run `cargo check`, `cargo check --target x86_64-unknown-linux-gnu`, `cargo test`
  outside the sandbox, `cargo clippy --all-targets -- -D warnings`, and `git diff --check`.
- Commit or push only when asked.

## Technical Defaults
- Keep the successful macOS heuristic/backend call chain and the Linux compiler path.
- Upgrade only the installed audit tool to registry-verified 0.22.2, using `--locked` and installation approval.

## Risks & Rollback
- macOS error text gains the existing test's required hint. Linux diagnostics keep their current path.
- No persisted data, public API, or format changes. Revert the validator edit to roll back.
- Reinstalling cargo-audit 0.20.0 rolls back tooling but restores its CVSS 4.0 parsing failure.

## Reproduction
- macOS, rustc 1.99.1, HEAD `819ae0043879a7eda932ef14351d944bc9e5714e` plus user changes.
- `cargo test`: 656 passed, 1 failed, 12 ignored. The missing-brace diagnostic regression fails.
- The same code exists at HEAD: `check(src)?` bypasses the shared error enrichment on macOS.
- `cargo audit` 0.20.0 fails parsing an advisory with `unsupported CVSS version: 4.0`.

## Task 1 — Restore shader hints and audit compatibility
**Files:** `src/render/postfx.rs:193` (`validate`), `src/render/postfx.rs:605` (tests).
**Interfaces:** Consumes and produces `validate(src: &str) -> Result<(), String>`;
the caller `load(paths: &[PathBuf], problems: &mut Vec<String>) -> (Vec<Source>, bool)` stays unchanged.

- [x] Run `cargo test render::postfx::tests::a_missing_brace_points_at_the_users_shader -- --exact`.
  Confirmed FAIL: actual `unbalanced braces or parentheses` lacks the user hint.
- [x] Add this regression and run `cargo test unbalanced_parentheses_point_at_the_users_shader`;
  expect FAIL for the same missing hint:

```rust
#[test]
fn unbalanced_parentheses_point_at_the_users_shader() {
    for (user, hint) in [
        ("void mainImage(out vec4 o, in vec2 f) { o = vec4(1.0; }", "unbalanced braces or parentheses"),
        ("void mainImage(out vec4 o, in vec2 f) { o = vec4(1.0)); }", "unmatched `)`"),
    ] {
        let err = validate(&wrap(user)).unwrap_err();
        assert!(err.contains(&format!("{hint} in your shader?")), "{err}");
    }
}
```

- [x] Replace `check(src)?` with platform-specific result bindings; keep the existing `map_err`:

```rust
#[cfg(target_os = "macos")]
let result = check(src).and_then(|()| crate::render::backend::validate_postfx(src));
#[cfg(not(target_os = "macos"))]
let result = crate::render::backend::validate_postfx(src);
// Existing hint enrichment applies to `result`.
```

- [x] Run `cargo test render::postfx::tests`; all 8 tests passed.
- [x] Install approved `cargo-audit` with `cargo install cargo-audit --version 0.22.2 --locked`;
  version 0.22.2 installed successfully. `cargo audit` exited 0 with 89 dependencies scanned,
  1,509 advisories loaded, and no vulnerabilities reported.
- Ready to commit: `src/render/postfx.rs`, this plan — `fix(render): preserve shader validation hints on macos`.

## Task 2 — Verification & Docs
- [x] Run all project verification commands above plus `cargo audit` and record results.
- [x] Run `cargo run -- +bench-perf` to exercise the upgraded parser in the actual CLI.
  Shader file loading is exercised by the existing `problems_name_the_file` test;
  desktop GPU shader rendering remains unverified unless exercised separately.
- [x] Read the full diff and untracked files; self-review requirements, error paths, concurrency,
  performance, security, compatibility, naming, tests, and design with concrete evidence.
- [x] Apply `code-review-strict`; independent review blocked by automatic approval review (see below).
- [x] No README/ROADMAP update: no config/keybinding or roadmap-item change.
- Ready to commit: the same files. No commit or push requested.

## Workflow limitations
- Both strict reviewer templates require `model: opus`, unavailable through the collaboration tool.
  The installed Claude CLI does support Opus, but automatic approval review rejected invoking it:
  sending repository diff/source context to an external service needs user authorization.
  No reviewer ran, no independent verdict exists, and no source was sent by that rejected command.
  Plan review before implementation was not performed; this process gap remains explicit.
- Execute inline under the user's fix request; no implementation-choice handoff is needed.

## Verification evidence and self-review
- Old missing-brace test and new parentheses test both failed before the production change.
- `cargo test render::postfx::tests`: 8 passed, including valid shaders, undeclared identifiers,
  file loading with missing/bad files, and missing/extra delimiters.
- `cargo check`: passed. Linux cross-check: passed. Clippy with all targets and denied warnings: passed.
- `git diff --check`: passed after source edit; rerun after this documentation update.
- CLI debug benchmark exited 1: performance thresholds failed (1 MiB parse 522.22 ms,
  stress 9,772 lines/s, worst stall 161.37 ms). Split/paste/probe/ringbuffer correctness checks passed.
  This is a recorded verification failure, not evidence of release performance or an attributed cause;
  no performance fix is included in this shader diagnostic change. GPU desktop rendering was not run.
- Requirements/edges: original regression plus missing/extra-parenthesis cases assert the actual hint.
- Error paths: the same validator error is retained and enriched; balanced-source errors are returned unchanged.
- Concurrency: the edit adds no shared state, locks, or I/O; the existing successful backend call remains.
- Performance: only config-load error formatting changes; parsing/render hot paths and Cell layout are untouched.
- Security/robustness: no shell execution, inputs, logging, secrets, or persisted formats added to production code.
- Naming/API/design: existing private validator signature, platform implementations, and shared formatter retained.
- No business rule defaulted; expected hint comes directly from the pre-existing failing regression test.
- Full `cargo test` outside sandbox: exit 0; 658 unit tests passed, 12 existing ignored;
  integration suites passed 1 and 14 tests. Total: 673 passed, 0 failed, 12 ignored.
- `cargo audit` 0.22.2: exit 0; 89 dependencies scanned, no vulnerabilities reported.
- Strict code review: 0 completed rounds, no verdict; external Opus invocation was rejected before execution.
  No reviewer findings were received, fixed, or rejected. User authorization remains necessary for that review.
- Inline batch complete; no integration menu, merge, commit, or push applied because none was requested.

## Publication update
- User subsequently requested: "commit open PR". Commit, push, and PR creation are now authorized.
- Include the existing VTE 0.15 / parking_lot 0.13 upgrade and its API adaptations together with
  the shader diagnostic regression fix, so the resulting commit passes the existing test contract.
- Fetch found a new `origin/main` commit; rebase the unpublished fix commit onto it, then rerun
  project verification before pushing. Keep the independent-review and benchmark gaps in the PR.
