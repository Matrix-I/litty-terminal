//! AI configuration is resolved only on `yo`, never on terminal startup.
use std::path::Path;

#[derive(Clone, Default, PartialEq)]
pub struct Settings(pub Vec<(String, String)>);
// Config debug output must never reveal credentials.
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AiSettings(<redacted>)")
    }
}
impl Settings {
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "provider" if !value.is_empty() => {
                Provider::parse(value)?;
            }
            "thinking"
                if !["", "off", "none", "minimal", "low", "medium", "high", "max"]
                    .contains(&value) =>
            {
                return Err("invalid ai-thinking".into());
            }
            "web-search" if !["", "true", "false"].contains(&value) => {
                return Err("ai-web-search must be true or false".into());
            }
            "context-window" if !value.is_empty() => {
                value
                    .parse::<usize>()
                    .ok()
                    .filter(|n| *n >= 4096)
                    .ok_or("ai-context-window must be at least 4096")?;
            }
            "provider" | "model" | "api-key" | "base-url" | "thinking" | "web-search"
            | "context-window" => {}
            _ => return Err(format!("unknown AI setting: ai-{key}")),
        }
        self.0.retain(|(k, _)| k != key);
        if !value.is_empty() {
            self.0.push((key.into(), value.into()));
        }
        Ok(())
    }
    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Anthropic,
    OpenAi,
    OpenRouter,
    DeepSeek,
    Kimi,
    Qwen,
    Zai,
    Ollama,
}
impl Provider {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "anthropic" => Ok(Self::Anthropic),
            "openai" => Ok(Self::OpenAi),
            "openrouter" => Ok(Self::OpenRouter),
            "deepseek" => Ok(Self::DeepSeek),
            "kimi" => Ok(Self::Kimi),
            "qwen" => Ok(Self::Qwen),
            "zai" | "z.ai" => Ok(Self::Zai),
            "ollama" => Ok(Self::Ollama),
            _ => Err(format!("unsupported AI provider: {s}")),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::OpenRouter => "openrouter",
            Self::DeepSeek => "deepseek",
            Self::Kimi => "kimi",
            Self::Qwen => "qwen",
            Self::Zai => "zai",
            Self::Ollama => "ollama",
        }
    }
    pub fn defaults(self) -> (&'static str, &'static str, usize) {
        match self {
            Self::Anthropic => (
                "https://api.anthropic.com/v1/messages",
                "claude-sonnet-4-5-20250929",
                200_000,
            ),
            Self::OpenAi => ("https://api.openai.com/v1/responses", "gpt-5.2", 400_000),
            Self::OpenRouter => (
                "https://openrouter.ai/api/v1/chat/completions",
                "anthropic/claude-sonnet-4.5",
                128_000,
            ),
            Self::DeepSeek => (
                "https://api.deepseek.com/chat/completions",
                "deepseek-chat",
                128_000,
            ),
            Self::Kimi => (
                "https://api.moonshot.ai/v1/chat/completions",
                "kimi-k2.5",
                128_000,
            ),
            Self::Qwen => (
                "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions",
                "qwen-plus",
                128_000,
            ),
            Self::Zai => (
                "https://api.z.ai/api/paas/v4/chat/completions",
                "glm-4.7",
                128_000,
            ),
            Self::Ollama => (
                "http://localhost:11434/v1/chat/completions",
                "qwen3",
                32_768,
            ),
        }
    }
    fn env(self) -> &'static str {
        match self {
            Self::Anthropic => "ANTHROPIC_API_KEY",
            Self::OpenAi => "OPENAI_API_KEY",
            Self::OpenRouter => "OPENROUTER_API_KEY",
            Self::DeepSeek => "DEEPSEEK_API_KEY",
            Self::Kimi => "KIMI_API_KEY",
            Self::Qwen => "QWEN_API_KEY",
            Self::Zai => "ZAI_API_KEY",
            Self::Ollama => "OLLAMA_API_KEY",
        }
    }
}
#[derive(Clone)]
pub struct Config {
    pub provider: Provider,
    pub model: String,
    pub key: String,
    pub url: String,
    pub thinking: String,
    pub web_search: bool,
    pub context_window: usize,
}
impl Config {
    pub fn load(settings: &Settings) -> Result<Self, String> {
        let home = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .ok_or("HOME is not set")?;
        Self::resolve(
            settings,
            &home,
            |k| std::env::var(k).ok(),
            |p| std::fs::read_to_string(p).ok(),
        )
    }
    fn resolve(
        settings: &Settings,
        home: &Path,
        env: impl Fn(&str) -> Option<String>,
        read: impl Fn(&Path) -> Option<String>,
    ) -> Result<Self, String> {
        let mut yosh = Settings::default();
        if let Some(text) = read(&home.join(".yoconf")) {
            for line in text.lines().map(str::trim).filter(|l| !l.starts_with('#')) {
                if let Some((k, v)) = line.split_once(char::is_whitespace) {
                    let k = match k {
                        "key" => "api-key",
                        "base_url" => "base-url",
                        "server_web" => "web-search",
                        "context_window" | "token_budget" => "context-window",
                        k => k,
                    };
                    let v = v.trim().trim_matches(['\'', '"']);
                    let v = if k == "web-search" {
                        match v {
                            "1" => "true",
                            "0" => "false",
                            _ => v,
                        }
                    } else {
                        v
                    };
                    let _ = yosh.set(k, v);
                }
            }
        }
        let explicit = settings.get("provider").or_else(|| yosh.get("provider"));
        let providers = [
            Provider::Anthropic,
            Provider::OpenAi,
            Provider::Kimi,
            Provider::DeepSeek,
            Provider::Qwen,
            Provider::Zai,
            Provider::OpenRouter,
        ];
        let keyfile = |p: Provider| {
            read(&home.join(format!(".{}key", p.name()))).filter(|s| !s.trim().is_empty())
        };
        let provider = if let Some(p) = explicit {
            Provider::parse(p)?
        } else {
            providers
                .into_iter()
                .find(|p| {
                    env(p.env()).is_some_and(|s| !s.trim().is_empty()) || keyfile(*p).is_some()
                })
                .unwrap_or(Provider::Anthropic)
        };
        // A provider override must not reuse another provider's Yosh key/model/URL.
        let yosh_provider = yosh
            .get("provider")
            .and_then(|p| Provider::parse(p).ok())
            .unwrap_or(Provider::Anthropic);
        let same = yosh_provider == provider;
        let get = |k| {
            settings
                .get(k)
                .or_else(|| if same { yosh.get(k) } else { None })
        };
        let (endpoint, model, window) = provider.defaults();
        let key = settings
            .get("api-key")
            .map(str::to_owned)
            .or_else(|| env(provider.env()).filter(|s| !s.trim().is_empty()))
            .or_else(|| {
                if same {
                    yosh.get("api-key").map(str::to_owned)
                } else {
                    None
                }
            })
            .or_else(|| keyfile(provider))
            .unwrap_or_default()
            .trim()
            .to_owned();
        if key.is_empty() && provider != Provider::Ollama {
            return Err(format!(
                "Set ai-provider and {} (or ai-api-key) before using yo.",
                provider.env()
            ));
        }
        let url = get("base-url")
            .map(|base| endpoint_url(base, provider))
            .unwrap_or_else(|| endpoint.into());
        if !(url.starts_with("https://")
            || url.starts_with("http://localhost:")
            || url.starts_with("http://127.0.0.1:")
            || url.starts_with("http://[::1]:"))
            || url.chars().any(char::is_control)
            || key.chars().any(char::is_control)
        {
            return Err("AI endpoint must use HTTPS (HTTP is allowed on loopback only); credentials cannot contain control characters".into());
        }
        Ok(Self {
            provider,
            model: get("model").unwrap_or(model).into(),
            key,
            url,
            thinking: get("thinking").unwrap_or("off").into(),
            web_search: get("web-search") == Some("true"),
            context_window: get("context-window")
                .and_then(|s| s.parse().ok())
                .unwrap_or(window),
        })
    }
}
fn endpoint_url(base: &str, p: Provider) -> String {
    let b = base.trim_end_matches('/');
    if b.ends_with("/messages") || b.ends_with("/responses") || b.ends_with("/chat/completions") {
        return b.into();
    }
    let suffix = match p {
        Provider::Anthropic => "messages",
        Provider::OpenAi => "responses",
        _ => "chat/completions",
    };
    format!("{b}/{suffix}")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn yosh_fallback_and_override() {
        let read = |p: &Path| match p.file_name().unwrap().to_str().unwrap() {
            ".yoconf" => Some("provider openai\nmodel legacy\nkey old".into()),
            _ => None,
        };
        let c = Config::resolve(&Settings::default(), Path::new("/fake"), |_| None, read).unwrap();
        assert_eq!(c.model, "legacy");
        let mut s = Settings::default();
        s.set("provider", "ollama").unwrap();
        let c = Config::resolve(&s, Path::new("/fake"), |_| None, read).unwrap();
        assert!(c.key.is_empty());
        assert_eq!(c.model, "qwen3");
    }
    #[test]
    fn env_detection_and_redaction() {
        let c = Config::resolve(
            &Settings::default(),
            Path::new("/fake"),
            |k| (k == "OPENAI_API_KEY").then(|| "secret".into()),
            |_| None,
        )
        .unwrap();
        assert_eq!(c.provider, Provider::OpenAi);
        let mut s = Settings::default();
        s.set("api-key", "secret").unwrap();
        assert!(!format!("{s:?}").contains("secret"));
    }
}
