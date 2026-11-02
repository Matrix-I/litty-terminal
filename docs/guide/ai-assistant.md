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

---

Back to the [README](../../README.md).
