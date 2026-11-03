//! Provider wire formats, preserving complete reasoning and tool-call messages.
use super::config::{Config, Provider};
use crate::json::{Json as J, obj};
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Command {
        command: String,
        explanation: String,
        pending: bool,
    },
    Chat {
        response: String,
    },
    Scrollback {
        lines: usize,
    },
    Docs,
}
#[derive(Debug)]
pub struct Call {
    pub id: String,
    pub action: Action,
}
pub struct Response {
    pub replay: Vec<J>,
    pub calls: Vec<Call>,
    pub text: String,
}
#[derive(Clone, Copy, PartialEq)]
pub enum Style {
    Anthropic,
    Responses,
    Chat,
}
pub fn style(c: &Config) -> Style {
    if c.provider == Provider::Anthropic {
        Style::Anthropic
    } else if c.url.ends_with("/responses") {
        Style::Responses
    } else {
        Style::Chat
    }
}
pub fn message(role: &str, text: &str) -> J {
    obj([("role", role.into()), ("content", text.into())])
}
fn schemas(s: Style) -> Vec<J> {
    let defs = [
        (
            "command",
            "Suggest one editable shell command",
            obj([
                ("command", obj([("type", "string".into())])),
                ("explanation", obj([("type", "string".into())])),
                ("pending", obj([("type", "boolean".into())])),
            ]),
        ),
        (
            "chat",
            "Answer the user in Markdown",
            obj([("response", obj([("type", "string".into())]))]),
        ),
        (
            "scrollback",
            "Read recent terminal output, up to 1000 lines",
            obj([(
                "lines",
                obj([
                    ("type", "integer".into()),
                    ("minimum", 1usize.into()),
                    ("maximum", 1000usize.into()),
                ]),
            )]),
        ),
        ("docs", "Read Litty yo configuration documentation", obj([])),
    ];
    defs.into_iter()
        .map(|(name, description, props)| {
            let required = if let J::Obj(p) = &props {
                J::Arr(p.iter().map(|(k, _)| k.clone().into()).collect())
            } else {
                J::Arr(vec![])
            };
            let schema = obj([
                ("type", "object".into()),
                ("properties", props),
                ("required", required),
                ("additionalProperties", false.into()),
            ]);
            match s {
                Style::Anthropic => obj([
                    ("name", name.into()),
                    ("description", description.into()),
                    ("input_schema", schema),
                ]),
                Style::Responses => obj([
                    ("type", "function".into()),
                    ("name", name.into()),
                    ("description", description.into()),
                    ("parameters", schema),
                    ("strict", true.into()),
                ]),
                Style::Chat => obj([
                    ("type", "function".into()),
                    (
                        "function",
                        obj([
                            ("name", name.into()),
                            ("description", description.into()),
                            ("parameters", schema),
                        ]),
                    ),
                ]),
            }
        })
        .collect()
}
pub fn build(c: &Config, system: &str, history: &[J], tools: bool) -> J {
    let s = style(c);
    let mut req = obj([("model", c.model.clone().into()), ("stream", false.into())]);
    match s {
        Style::Anthropic => {
            req.set("max_tokens", 8192usize.into());
            req.set(
                "system",
                J::Arr(vec![obj([
                    ("type", "text".into()),
                    ("text", system.into()),
                    ("cache_control", obj([("type", "ephemeral".into())])),
                ])]),
            );
            let mut history = history.to_vec();
            // Cache the end of the stable conversation prefix.
            if let Some(last) = history.last_mut()
                && let Some(J::Str(text)) = last.get("content").cloned()
            {
                last.set(
                    "content",
                    J::Arr(vec![obj([
                        ("type", "text".into()),
                        ("text", text.into()),
                        ("cache_control", obj([("type", "ephemeral".into())])),
                    ])]),
                );
            }
            req.set("messages", J::Arr(history));
            if !["off", "none"].contains(&c.thinking.as_str()) {
                req.set(
                    "thinking",
                    obj([
                        ("type", "enabled".into()),
                        (
                            "budget_tokens",
                            match c.thinking.as_str() {
                                "minimal" | "low" => 1024usize,
                                "medium" => 2048,
                                "high" => 4096,
                                _ => 6144,
                            }
                            .into(),
                        ),
                    ]),
                );
            }
        }
        Style::Responses => {
            req.set("instructions", system.into());
            req.set("input", J::Arr(history.to_vec()));
            req.set("store", false.into());
            req.set("max_output_tokens", 8192usize.into());
            req.set("prompt_cache_key", "litty".into());
            req.set(
                "include",
                J::Arr(vec!["reasoning.encrypted_content".into()]),
            );
            if let Some(effort) = reasoning(c) {
                req.set("reasoning", obj([("effort", effort.into())]));
            }
        }
        Style::Chat => {
            let mut messages = vec![message("system", system)];
            messages.extend_from_slice(history);
            if c.provider == Provider::OpenRouter && c.model.starts_with("qwen/") {
                mark_cache(&mut messages[0]);
                if let Some(last) = messages.iter_mut().rev().find(|m| m.text("role") == "user") {
                    mark_cache(last);
                }
            }
            req.set("messages", J::Arr(messages));
            if let Some(effort) = reasoning(c) {
                match c.provider {
                    Provider::OpenRouter => req.set("reasoning", obj([("effort", effort.into())])),
                    Provider::DeepSeek | Provider::Kimi | Provider::Zai => {
                        req.set("thinking", obj([("type", "enabled".into())]))
                    }
                    Provider::Qwen => req.set("enable_thinking", true.into()),
                    _ => req.set("reasoning_effort", effort.into()),
                }
            }
        }
    }
    if tools {
        let mut list = schemas(s);
        if s == Style::Anthropic
            && let Some(t) = list.last_mut()
        {
            t.set("cache_control", obj([("type", "ephemeral".into())]));
        }
        if c.web_search {
            match s {
                Style::Anthropic => list.push(obj([
                    ("type", "web_search_20250305".into()),
                    ("name", "web_search".into()),
                ])),
                Style::Responses => list.push(obj([("type", "web_search".into())])),
                Style::Chat => {}
            }
        }
        req.set("tools", J::Arr(list));
        if s != Style::Anthropic {
            req.set("parallel_tool_calls", false.into());
        }
        if c.provider == Provider::OpenRouter && c.model.starts_with("anthropic/") {
            req.set("cache_control", obj([("type", "ephemeral".into())]));
        }
    }
    req
}
fn reasoning(c: &Config) -> Option<&str> {
    let off = ["off", "none"].contains(&c.thinking.as_str());
    if c.provider == Provider::OpenAi && c.model.starts_with("gpt-5.2") {
        return Some(match c.thinking.as_str() {
            "off" | "none" => "none",
            "minimal" => "low",
            "max" => "xhigh",
            other => other,
        });
    }
    if off {
        None
    } else {
        Some(match c.thinking.as_str() {
            "max" => "high",
            other => other,
        })
    }
}
fn mark_cache(message: &mut J) {
    if let Some(J::Str(text)) = message.get("content").cloned() {
        message.set(
            "content",
            J::Arr(vec![obj([
                ("type", "text".into()),
                ("text", text.into()),
                ("cache_control", obj([("type", "ephemeral".into())])),
            ])]),
        );
    }
}
pub fn headers(c: &Config) -> Vec<String> {
    let mut h = vec!["Content-Type: application/json".into()];
    if c.provider == Provider::Anthropic {
        h.push(format!("x-api-key: {}", c.key));
        h.push("anthropic-version: 2023-06-01".into());
    } else if !c.key.is_empty() {
        h.push(format!("Authorization: Bearer {}", c.key));
    }
    h
}
fn action(name: &str, args: &J) -> Result<Action, String> {
    let required = |key| {
        args.get(key)
            .and_then(J::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("AI {name} tool missing {key}"))
    };
    match name {
        "command" => {
            let command = required("command")?;
            if command.is_empty() || command.len() > 64 * 1024 {
                return Err("AI command is empty or too large".into());
            }
            Ok(Action::Command {
                command,
                explanation: required("explanation")?,
                pending: args
                    .get("pending")
                    .and_then(J::boolean)
                    .ok_or("AI command missing pending")?,
            })
        }
        "chat" => Ok(Action::Chat {
            response: required("response")?,
        }),
        "scrollback" => Ok(Action::Scrollback {
            lines: args
                .get("lines")
                .and_then(J::usize)
                .ok_or("AI scrollback missing lines")?
                .clamp(1, 1000),
        }),
        "docs" => Ok(Action::Docs),
        _ => Err(format!("Unknown AI tool: {name}")),
    }
}
pub fn parse(s: Style, body: &str) -> Result<Response, String> {
    let root = J::parse(body)?;
    if root.get("error").is_some_and(|e| *e != J::Null) {
        return Err("AI provider returned an error; use yo show last response".into());
    }
    let mut response = Response {
        replay: vec![],
        calls: vec![],
        text: String::new(),
    };
    match s {
        Style::Anthropic => {
            let content = root.get("content").ok_or("Missing Anthropic content")?;
            for b in content.array() {
                match b.text("type") {
                    "tool_use" => response.calls.push(Call {
                        id: b.text("id").into(),
                        action: action(
                            b.text("name"),
                            b.get("input").ok_or("Missing tool input")?,
                        )?,
                    }),
                    "text" => response.text.push_str(b.text("text")),
                    _ => {}
                }
            }
            response.replay.push(obj([
                ("role", "assistant".into()),
                ("content", content.clone()),
            ]));
        }
        Style::Responses => {
            for b in root
                .get("output")
                .ok_or("Missing Responses output")?
                .array()
            {
                response.replay.push(b.clone());
                match b.text("type") {
                    "function_call" => response.calls.push(Call {
                        id: b.text("call_id").into(),
                        action: action(b.text("name"), &J::parse(b.text("arguments"))?)?,
                    }),
                    "message" => {
                        for c in b.get("content").map(J::array).unwrap_or_default() {
                            if c.text("type") == "output_text" {
                                response.text.push_str(c.text("text"));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Style::Chat => {
            let m = root
                .get("choices")
                .and_then(|c| c.array().first())
                .and_then(|c| c.get("message"))
                .ok_or("Missing chat completion")?;
            response.text = m.text("content").into();
            response.replay.push(m.clone());
            for c in m.get("tool_calls").map(J::array).unwrap_or_default() {
                let f = c.get("function").ok_or("Missing function")?;
                response.calls.push(Call {
                    id: c.text("id").into(),
                    action: action(f.text("name"), &J::parse(f.text("arguments"))?)?,
                });
            }
        }
    }
    if response.calls.iter().any(|c| c.id.is_empty()) {
        return Err("AI tool call has no ID".into());
    }
    Ok(response)
}
pub fn tool_results(s: Style, results: Vec<(String, String)>) -> Vec<J> {
    match s {
        Style::Anthropic => vec![obj([
            ("role", "user".into()),
            (
                "content",
                J::Arr(
                    results
                        .into_iter()
                        .map(|(id, text)| {
                            obj([
                                ("type", "tool_result".into()),
                                ("tool_use_id", id.into()),
                                ("content", text.into()),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])],
        Style::Responses => results
            .into_iter()
            .map(|(id, text)| {
                obj([
                    ("type", "function_call_output".into()),
                    ("call_id", id.into()),
                    ("output", text.into()),
                ])
            })
            .collect(),
        Style::Chat => results
            .into_iter()
            .map(|(id, text)| {
                obj([
                    ("role", "tool".into()),
                    ("tool_call_id", id.into()),
                    ("content", text.into()),
                ])
            })
            .collect(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn c(p: Provider) -> Config {
        let (u, m, w) = p.defaults();
        Config {
            provider: p,
            url: u.into(),
            model: m.into(),
            key: "secret".into(),
            thinking: "off".into(),
            web_search: false,
            context_window: w,
        }
    }
    #[test]
    fn builds_three_wire_styles() {
        for p in [Provider::Anthropic, Provider::OpenAi, Provider::Ollama] {
            let c = c(p);
            let r = build(&c, "system", &[message("user", "hi")], true);
            assert_eq!(J::parse(&r.encode()).unwrap(), r);
            assert_eq!(r.get("tools").unwrap().array().len(), 4);
            assert!(!r.encode().contains("secret"));
        }
    }
    #[test]
    fn parses_tools_all_styles() {
        for (s, b) in [
            (
                Style::Anthropic,
                r#"{"content":[{"type":"thinking","thinking":"think","signature":"sig"},{"type":"tool_use","id":"1","name":"docs","input":{}}]}"#,
            ),
            (
                Style::Responses,
                r#"{"output":[{"type":"reasoning","encrypted_content":"abc"},{"type":"function_call","call_id":"1","name":"docs","arguments":"{}"}]}"#,
            ),
            (
                Style::Chat,
                r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"1","function":{"name":"docs","arguments":"{}"}}]}}]}"#,
            ),
        ] {
            let r = parse(s, b).unwrap();
            assert_eq!(r.calls[0].action, Action::Docs);
            assert!(!r.replay.is_empty());
            assert!(!tool_results(s, vec![("1".into(), "done".into())]).is_empty());
        }
    }
    #[test]
    fn bounded_scrollback_and_command_validation() {
        assert_eq!(
            action("scrollback", &obj([("lines", 99999usize.into())])).unwrap(),
            Action::Scrollback { lines: 1000 }
        );
        assert!(action("command", &obj([])).is_err());
    }
}
