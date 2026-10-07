use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct GeminiConfig {
    /// `gemini-3.8-flash-tts`, or `gemini-3.8-flash-lite-tts` for many
    /// lines at less cost.
    pub model: String,
    /// The environment variable holding the API key. Read when a line is
    /// spoken, so `check` and `plan` need none.
    pub api_key_env: String,
    /// The API's address; the real one unless testing.
    pub base_url: String,
    pub timeout_ms: u64,
    /// Lines `dub` sends at once; a free tier allows few requests a minute.
    pub concurrency: usize,
    /// Sent with every line when set, for the same line to sound the same.
    pub seed: Option<u64>,
}

impl Default for GeminiConfig {
    fn default() -> Self {
        Self {
            model: "gemini-3.8-flash-tts".to_string(),
            api_key_env: "GEMINI_API_KEY".to_string(),
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            timeout_ms: 120_000,
            concurrency: 4,
            seed: None,
        }
    }
}

impl GeminiConfig {
    pub fn from_value(v: &serde_json::Value) -> Result<Self, String> {
        let mut c: GeminiConfig = serde_json::from_value(v.clone())
            .map_err(|e| format!("invalid `backends.gemini` settings: {e}"))?;
        while c.base_url.ends_with('/') {
            c.base_url.pop();
        }
        for (name, value) in [
            ("model", &c.model),
            ("api_key_env", &c.api_key_env),
            ("base_url", &c.base_url),
        ] {
            if value.is_empty() {
                return Err(format!("`backends.gemini.{name}` must not be empty"));
            }
        }
        if c.concurrency == 0 {
            return Err("`backends.gemini.concurrency` must be at least 1".to_string());
        }
        Ok(c)
    }

    /// What changes the audio beyond the request (docs/design.md#voice-cache):
    /// the model and the seed.
    pub fn version_string(&self) -> String {
        match self.seed {
            Some(seed) => format!("gemini/{}/seed{seed}", self.model),
            None => format!("gemini/{}", self.model),
        }
    }
}
