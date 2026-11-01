#!/usr/bin/env bash
# Real API smoke test. Only synthetic terminal content is sent to Anthropic.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "${1:-}" == "--help" ]]; then
  printf '%s\n' 'Usage: bash scripts/test-ai-live.sh' \
    'Optional: LITTY_AI_LIVE_MODEL=<accessible Anthropic model ID>' \
    'Uses ANTHROPIC_API_KEY if set, otherwise prompts without echo.' \
    'Makes real, billable API requests using synthetic fixtures only.' \
    'Does not modify ~/.litty/config or execute model-generated commands.'
  exit 0
fi

# Prepare the harness before asking for a credential.
cargo test ai::live_tests::anthropic_live_smoke --no-run
if [[ -z "${ANTHROPIC_API_KEY:-}" ]]; then
  if [[ ! -t 0 ]]; then
    printf '%s\n' 'No interactive terminal: set ANTHROPIC_API_KEY securely in the calling environment.' >&2
    exit 2
  fi
  IFS= read -r -s -p 'Anthropic API key (hidden): ' ANTHROPIC_API_KEY
  printf '\n'
fi
if [[ -z "$ANTHROPIC_API_KEY" ]]; then
  printf '%s\n' 'API key is empty; no request made.' >&2
  exit 2
fi
export ANTHROPIC_API_KEY
export LITTY_AI_LIVE_TEST=1
export LITTY_AI_LIVE_MODEL="${LITTY_AI_LIVE_MODEL:-claude-sonnet-5-5}"
trap 'unset ANTHROPIC_API_KEY' EXIT
cargo test ai::live_tests::anthropic_live_smoke -- --ignored --nocapture --test-threads=1
