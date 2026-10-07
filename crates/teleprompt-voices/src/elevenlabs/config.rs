use serde::Deserialize;

/// `[backends.elevenlabs]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ElevenLabsConfig {
    /// `eleven_multilingual_v2`, `eleven_v3`, `eleven_flash_v2_5`, …
    pub model: String,
    /// The environment variable holding the API key. Read when a line is
    /// spoken, so `check` and `plan` need none.
    pub api_key_env: String,
    /// The API's address; the real one unless testing.
    pub base_url: String,
    pub timeout_ms: u64,
    /// Lines `dub` sends at once; a plan allows only so many at a time.
    pub concurrency: usize,
    /// Sent with every line when set, for the same line to sound the same.
    pub seed: Option<u64>,
    /// The voice's own settings, each from 0 to 1, sent when set.
    pub stability: Option<f64>,
    pub similarity_boost: Option<f64>,
    pub style: Option<f64>,
}

impl Default for ElevenLabsConfig {
    fn default() -> Self {
        Self {
            model: "eleven_multilingual_v2".to_string(),
            api_key_env: "ELEVENLABS_API_KEY".to_string(),
            base_url: "https://api.elevenlabs.io".to_string(),
            timeout_ms: 60_000,
            concurrency: 2,
            seed: None,
            stability: None,
            similarity_boost: None,
            style: None,
        }
    }
}

impl ElevenLabsConfig {
    pub fn from_value(v: &serde_json::Value) -> Result<Self, String> {
        let mut c: ElevenLabsConfig = serde_json::from_value(v.clone())
            .map_err(|e| format!("invalid `backends.elevenlabs` settings: {e}"))?;
        while c.base_url.ends_with('/') {
            c.base_url.pop();
        }
        for (name, value) in [
            ("model", &c.model),
            ("api_key_env", &c.api_key_env),
            ("base_url", &c.base_url),
        ] {
            if value.is_empty() {
                return Err(format!("`backends.elevenlabs.{name}` must not be empty"));
            }
        }
        if c.concurrency == 0 {
            return Err("`backends.elevenlabs.concurrency` must be at least 1".to_string());
        }
        for (name, value) in [
            ("stability", c.stability),
            ("similarity_boost", c.similarity_boost),
            ("style", c.style),
        ] {
            if value.is_some_and(|v| !(0.0..=1.0).contains(&v)) {
                return Err(format!("`backends.elevenlabs.{name}` is from 0 to 1"));
            }
        }
        Ok(c)
    }

    /// What changes the audio beyond the request (docs/design.md#voice-cache):
    /// the model, the seed and the voice settings.
    pub fn version_string(&self) -> String {
        let mut v = format!("elevenlabs/{}", self.model);
        if let Some(seed) = self.seed {
            v.push_str(&format!("/seed{seed}"));
        }
        for (name, value) in [
            ("stability", self.stability),
            ("similarity", self.similarity_boost),
            ("style", self.style),
        ] {
            if let Some(value) = value {
                v.push_str(&format!("/{name}{value}"));
            }
        }
        v
    }
}
