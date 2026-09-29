//! A model run locally by Ollama (ollama.com), the default: nothing leaves
//! the machine and nothing needs a key.

use serde::Deserialize;
use serde_json::{json, Value};
use teleprompt_core::error::with_causes;

use crate::prompt::{answer, schema, SYSTEM};
use crate::{Request, Response};

/// A multilingual open model that runs on a laptop.
pub const DEFAULT_MODEL: &str = "gemma3:12b";

/// Where Ollama listens unless `OLLAMA_HOST` or the settings say otherwise.
const DEFAULT_URL: &str = "http://localhost:11434";

/// `[backends.ollama]`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub url: Option<String>,
}

pub struct Ollama {
    pub url: String,
    pub model: String,
    pub timeout_ms: u64,
}

impl Ollama {
    pub fn new(model: Option<&str>, settings: Settings) -> Self {
        let url = settings
            .url
            .or_else(|| std::env::var("OLLAMA_HOST").ok().map(|h| with_scheme(&h)))
            .unwrap_or_else(|| DEFAULT_URL.to_string());
        Ollama {
            url,
            model: model.unwrap_or(DEFAULT_MODEL).to_string(),
            timeout_ms: crate::TIMEOUT_MS,
        }
    }

    pub(crate) async fn translate(&self, request: &Request) -> Result<Response, String> {
        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": SYSTEM },
                { "role": "user", "content": serde_json::to_string_pretty(request).map_err(|e| e.to_string())? },
            ],
            "format": schema(),
            "stream": false,
            // The same words for the same line, run after run.
            "options": { "temperature": 0 },
        });
        let reply = crate::client(self.timeout_ms)
            .post(format!("{}/api/chat", self.url.trim_end_matches('/')))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    return crate::unanswered("Ollama", self.timeout_ms);
                }
                format!(
                    "cannot reach Ollama at {}: {}\n  install it from ollama.com, then \
                     `ollama pull {}`; or choose another provider under [translate]",
                    self.url,
                    with_causes(&e),
                    self.model
                )
            })?;
        let status = reply.status();
        let message: Value = reply.json().await.map_err(|e| {
            if e.is_timeout() {
                return crate::unanswered("Ollama", self.timeout_ms);
            }
            format!("Ollama answered {status} with no JSON: {e}")
        })?;
        if let Some(error) = message["error"].as_str() {
            let help = if error.contains("not found") {
                format!("\n  run `ollama pull {}`", self.model)
            } else {
                String::new()
            };
            return Err(format!("Ollama: {error}{help}"));
        }
        answer(
            message["message"]["content"].as_str().unwrap_or_default(),
            "the model",
        )
    }
}

/// `OLLAMA_HOST` as Ollama reads it: `host:port` means http.
fn with_scheme(host: &str) -> String {
    if host.contains("://") {
        host.to_string()
    } else {
        format!("http://{host}")
    }
}
