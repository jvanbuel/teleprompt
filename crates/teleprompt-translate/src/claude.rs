//! Claude as a translator, through the Messages API. Rust has no official
//! Anthropic SDK, so this is the documented HTTP request.

use serde_json::{json, Value};

use crate::prompt::{answer, schema, SYSTEM};
use crate::{Request, Response};

/// The model asked unless the author names another.
pub const DEFAULT_MODEL: &str = "claude-opus-5";

const API: &str = "https://api.anthropic.com";

/// Enough for a batch of lines translated in full.
const MAX_TOKENS: u32 = 16_000;

/// Claude, reached with an API key.
pub struct Claude {
    pub api_key: String,
    pub model: String,
    /// The API's address; the real one unless testing.
    pub base_url: String,
}

impl Claude {
    /// Credentials and endpoint from the environment: `ANTHROPIC_API_KEY`,
    /// and `ANTHROPIC_BASE_URL` if set.
    pub fn from_env(model: Option<&str>) -> Result<Self, String> {
        let api_key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
            "translating with Claude needs an Anthropic API key in ANTHROPIC_API_KEY \
             (console.anthropic.com); the default provider, ollama, needs none"
                .to_string()
        })?;
        Ok(Claude {
            api_key,
            model: model.unwrap_or(DEFAULT_MODEL).to_string(),
            base_url: std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| API.to_string()),
        })
    }

    pub(crate) async fn translate(&self, request: &Request) -> Result<Response, String> {
        let body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            // A declined request is re-run on the model Anthropic
            // recommends for it, rather than failing the translation.
            "fallbacks": "default",
            "system": SYSTEM,
            "output_config": { "format": { "type": "json_schema", "schema": schema() } },
            "messages": [{
                "role": "user",
                "content": serde_json::to_string_pretty(request).map_err(|e| e.to_string())?,
            }],
        });
        let reply = crate::client(crate::TIMEOUT_MS)
            .post(format!(
                "{}/v1/messages",
                self.base_url.trim_end_matches('/')
            ))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "server-side-fallback-2026-07-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    return format!(
                        "the Anthropic API did not answer within {} ms",
                        crate::TIMEOUT_MS
                    );
                }
                format!("cannot reach the Anthropic API: {e}")
            })?;
        let status = reply.status();
        let message: Value = reply
            .json()
            .await
            .map_err(|e| format!("the Anthropic API answered {status} with no JSON: {e}"))?;
        if !status.is_success() {
            let why = message["error"]["message"]
                .as_str()
                .unwrap_or("no reason given");
            return Err(format!(
                "the Anthropic API refused the request ({status}): {why}"
            ));
        }
        read(&message)
    }
}

/// The translations in a Messages API reply.
fn read(message: &Value) -> Result<Response, String> {
    match message["stop_reason"].as_str() {
        Some("refusal") => {
            let why = message["stop_details"]["explanation"]
                .as_str()
                .unwrap_or("");
            return Err(format!("Claude declined to translate this script. {why}"));
        }
        Some("max_tokens") => {
            return Err("Claude's answer was cut off before it finished".to_string());
        }
        _ => {}
    }
    let text: String = message["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect();
    answer(&text, "Claude")
}
