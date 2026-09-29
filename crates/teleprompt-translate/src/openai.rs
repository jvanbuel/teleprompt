//! Any server speaking the OpenAI chat completions API: LM Studio,
//! llama.cpp's server, vLLM, LocalAI, or a hosted service.

use serde::Deserialize;
use serde_json::{json, Value};
use teleprompt_core::error::with_causes;

use crate::prompt::{answer, schema, SYSTEM};
use crate::{Request, Response};

/// `[backends.openai]`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// The API's base, up to `/v1`: `http://localhost:1234/v1` for LM Studio.
    pub url: Option<String>,
    /// The environment variable holding a key, for a server that wants one.
    pub api_key_env: Option<String>,
    pub timeout_ms: Option<u64>,
}

pub struct OpenAi {
    pub url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub timeout_ms: u64,
}

/// Everything but the key, which would otherwise end up in a log.
impl std::fmt::Debug for OpenAi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAi")
            .field("url", &self.url)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

impl OpenAi {
    pub fn new(model: Option<&str>, settings: Settings) -> Result<Self, String> {
        let url = settings.url.ok_or(
            "the openai provider needs the server's address: set `url` under [backends.openai], \
             e.g. http://localhost:1234/v1 for LM Studio",
        )?;
        let model =
            model.ok_or("the openai provider needs a model: set `model` under [translate]")?;
        let api_key = match settings.api_key_env {
            Some(var) => Some(std::env::var(&var).map_err(|_| format!("{var} is not set"))?),
            None => None,
        };
        Ok(OpenAi {
            url,
            model: model.to_string(),
            api_key,
            timeout_ms: settings.timeout_ms.unwrap_or(crate::TIMEOUT_MS),
        })
    }

    pub(crate) async fn translate(&self, request: &Request) -> Result<Response, String> {
        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": SYSTEM },
                { "role": "user", "content": serde_json::to_string_pretty(request).map_err(|e| e.to_string())? },
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": "translation", "strict": true, "schema": schema() },
            },
            "temperature": 0,
        });
        let mut post = crate::client(self.timeout_ms)
            .post(format!(
                "{}/chat/completions",
                self.url.trim_end_matches('/')
            ))
            .json(&body);
        if let Some(key) = &self.api_key {
            post = post.bearer_auth(key);
        }
        let reply = post.send().await.map_err(|e| {
            if e.is_timeout() {
                return crate::unanswered(&self.url, self.timeout_ms, "openai");
            }
            format!("cannot reach {}: {}", self.url, with_causes(&e))
        })?;
        let status = reply.status();
        let message: Value = reply.json().await.map_err(|e| {
            if e.is_timeout() {
                return crate::unanswered(&self.url, self.timeout_ms, "openai");
            }
            format!("{} answered {status} with no JSON: {e}", self.url)
        })?;
        if !status.is_success() {
            let why = message["error"]["message"]
                .as_str()
                .or_else(|| message["error"].as_str())
                .unwrap_or("no reason given");
            return Err(format!(
                "{} refused the request ({status}): {why}",
                self.url
            ));
        }
        answer(
            message["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or_default(),
            "the model",
        )
    }
}
