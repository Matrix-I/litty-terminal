# AI Shell Assistance (`yo`)

With shell integration enabled, type `yo <task or question>` in zsh, bash, fish, or nushell. The assistant can suggest a command in your editable prompt, answer inline, or inspect recent terminal output. Commands run when you press Enter. Ctrl+C cancels thinking; typing, paste, and Enter during thinking wait in a per-pane queue until the response and prompt are ready. An Enter queued during thinking can run the suggestion once it appears.

For example, add this to `~/.litty/config` and set `OPENAI_API_KEY` in Litty's environment:

```ini
ai-provider = openai
ai-model = gpt-5.2
ai-thinking = off
ai-web-search = false
```

Providers are `anthropic`, `openai`, `openrouter`, `deepseek`, `kimi`, `qwen`, `zai`, and `ollama`. OpenAI uses Responses by default; a full `ai-base-url` ending in `/chat/completions` selects Chat Completions. For Ollama, use `ai-provider = ollama` and `ai-model = <your installed model>`; no key is required. `ai-base-url` accepts an API root such as `http://localhost:11434/v1` or a complete endpoint. Custom models can set `ai-context-window` in tokens.

Keys resolve from `ai-api-key`, the selected provider's environment variable (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `DEEPSEEK_API_KEY`, `KIMI_API_KEY`, `QWEN_API_KEY`, or `ZAI_API_KEY`), Yosh `~/.yoconf`, or the matching `~/.<provider>key` file. Explicit Litty provider/model/endpoint settings override Yosh fallback values. Configuration is read when a query starts. Thinking and web search are off by default; server web search is available through Anthropic and Responses endpoints.

Multi-step suggestions still require Enter for every command. Editing a pending suggestion continues the workflow with the command actually executed and its output. Empty Enter, Ctrl+C, or a new `yo` query cancels the previous chain. Model-requested scrollback and continuation output are sent to the configured provider; sessions and the last response stay in memory per pane. Local commands are `yo reset`, `yo show last response`, and `yo show documentation`.

## Trigger word

`ai-trigger` sets the word that starts a query (default `yo`). It must be one shell word of letters, digits, `_` and `-` (at most 32 characters, not starting with a digit or `-`); `ai-trigger = off` turns AI invocation off, and an empty value restores the default. The Rust side matches it live; the shell scripts read it once when a shell starts, so open a new tab or split after changing it. With another trigger or none, `yo` is an ordinary command again (for example Yeoman's `yo generator` runs the real executable). Nushell cannot define a command under a chosen name, so a renamed trigger there works only by Litty's own Enter interception, and its `yo` function forwards to the real `yo`.

## Redaction before sending

Everything sent to the provider (queries, continuation output, scrollback the model requests, recorded executed commands and compaction requests) is first filtered. Matches are replaced with `[REDACTED:kind]` and the status line says how many were removed. Covered: PEM private key blocks (a truncated block is cut to the end), AWS access key ids, GitHub tokens, `Bearer` tokens, and `name=value` / `name: value` assignments whose name contains password, passwd, secret, token, api key or access key and whose value has at least four characters. This is a bounded safety net: other secret shapes, values passed as `--flag value`, and short values are not detected, so do not rely on it for sensitive output.

---

Back to the [README](../../README.md).
