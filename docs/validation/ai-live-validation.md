# Anthropic live-provider validation

Status: runner prepared; **not yet validated against the live API**. No Anthropic
credential was available during preparation. Normal `cargo test` skips this test.

Preparation checks: shell syntax/help, macOS and Linux cross-target compilation,
Clippy with zero warnings, and the full sequential suite outside the sandbox
(302 passed, 5 ignored). The live test is one of those ignored tests; these checks
do not imply that Anthropic requests have succeeded.

Run from the repository root in an interactive terminal:

```sh
bash scripts/test-ai-live.sh
```

The script compiles the test harness, then asks for an API key without echoing it.
An existing `ANTHROPIC_API_KEY` is also supported. The key is kept in the runner's
environment and is not written into configuration, command arguments or reports.
Do not paste keys into chat. Requests go to `https://api.anthropic.com/v1/messages`.
The test makes real, billable requests using synthetic fixtures only.

The runner defaults to `claude-sonnet-5-5`, a model ID listed in the
[Anthropic model overview](https://platform.claude.com/docs/en/models/overview).
If your account needs a different model, specify its ID:

```sh
LITTY_AI_LIVE_MODEL=your-accessible-model-id bash scripts/test-ai-live.sh
```

The model is selected for this test only. The production default and
`~/.litty/config` remain unchanged.

Expected output includes four PASS groups and a successful Rust test result:

1. Real docs and scrollback tool round trips followed by chat. Only an isolated
   terminal fixture is supplied, with a known marker to verify the answer.
2. A pending command suggestion, an edited execution report and a continuation
   acknowledging the actual output. The test executes a fixed `/usr/bin/printf`
   fixture; it never executes model-generated text.
3. Real context summarization at a deliberately small 4096-token test threshold,
   followed by a successful request using the compacted session.
4. Cancellation of the curl request after about 200 ms, with local cleanup
   completing within two seconds. This does not claim server-side cancellation
   or reversal of any provider charges.

Each regular test turn has a 90-second cancellation deadline. A failed HTTP request
prints its status code without dumping request/response bodies or credentials.
The runner uses the production transport, provider serializer/parser, tool loop,
session execution records and compaction implementation.

This is not a GPU-window interaction test. Actual keyboard Ctrl+C, queued Enter,
prompt repaint and editable prefill still need real-app coverage; deterministic
lifecycle tests and the separate PTY shell smoke tests cover those components.
Provider/model compliance failures are reported as failures, not skipped passes.

After running, share only the PASS/FAIL lines and any HTTP status to record the
result. Do not include credentials or raw provider responses. The roadmap remains
open until the live run has succeeded and the remaining release checks are done.
