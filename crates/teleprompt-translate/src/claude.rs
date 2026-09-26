//! Claude as a translator, through the Messages API. Rust has no official
//! Anthropic SDK, so this is the documented HTTP request.

use serde_json::{json, Value};

use crate::{Request, Response};

/// The model asked unless the author names another.
pub const DEFAULT_MODEL: &str = "claude-opus-5";

const API: &str = "https://api.anthropic.com";

/// Enough for a batch of lines translated in full.
const MAX_TOKENS: u32 = 16_000;

const SYSTEM: &str = "\
You translate the narration of a narrated software video. Each line is \
spoken aloud by a text-to-speech voice over a screen recording, so write \
what a native speaker would say: natural, spoken, and about as long as the \
original, since the pictures are timed to it.

Keep product names, commands, code, file names, flags and keys exactly as \
written. Keep the terminology of the translations already made. A chapter \
is a short heading.

A cue is a phrase of a line that a shot starts on. For each cue, answer \
with the words of your translation of that line (given by `line`, or in \
`existing` when it is not being translated now) that say the same thing, \
copied exactly, character for character. When the phrase is a command \
kept as written, answer with it unchanged.

Answer every item in `translate`, by its `id`.";

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
             (console.anthropic.com), or translate with --with command"
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
        let reply = reqwest::Client::new()
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
            .map_err(|e| format!("cannot reach the Anthropic API: {e}"))?;
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
    serde_json::from_str(&text)
        .map_err(|e| format!("Claude's answer is not the JSON asked for: {e}"))
}

/// The answer's shape: `{"items": [{"id", "text"}]}`.
fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string" },
                        "text": { "type": "string" },
                    },
                    "required": ["id", "text"],
                    "additionalProperties": false,
                },
            },
        },
        "required": ["items"],
        "additionalProperties": false,
    })
}
