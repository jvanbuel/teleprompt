use serde::Deserialize;

/// Kokoro-FastAPI emits 24 kHz mono. `Pcm` carries the rate, the WAV
/// encoder writes any rate, and the manifest reports whatever the backend
/// produced — so this constant is the backend's own fact, not a global one.
pub const KOKORO_SAMPLE_RATE: u32 = 24_000;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct KokoroConfig {
    pub base_url: String,
    pub timeout_ms: u64,
    /// Bounded fan-out for `dub`. A local model server is the bottleneck and
    /// unbounded concurrency makes it slower, not faster.
    pub concurrency: usize,
    /// Sent as the request's `model` field and folded into the cache key.
    pub model: String,
}

impl Default for KokoroConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:8880".to_string(),
            timeout_ms: 30_000,
            concurrency: 4,
            model: "kokoro".to_string(),
        }
    }
}

impl KokoroConfig {
    pub fn from_value(v: &serde_yaml::Value) -> Result<Self, String> {
        let mut c: KokoroConfig = serde_yaml::from_value(v.clone())
            .map_err(|e| format!("invalid `backends.kokoro` settings: {e}"))?;
        while c.base_url.ends_with('/') {
            c.base_url.pop();
        }
        if c.base_url.is_empty() {
            return Err("`backends.kokoro.base_url` must not be empty".to_string());
        }
        if c.concurrency == 0 {
            return Err("`backends.kokoro.concurrency` must be at least 1".to_string());
        }
        Ok(c)
    }

    /// What `VoiceCapabilities::version` returns, and therefore part of every
    /// cache key this backend's audio is stored under.
    ///
    /// Host and model are both in it because both change the audio while
    /// leaving the `SynthRequest` identical: two servers with different
    /// checkpoints answer the same request differently, and nothing above
    /// the cache could tell. The contract doc on
    /// `VoiceCapabilities::version` names this exact hazard.
    ///
    /// The cost is a real one and worth stating: `localhost` and `127.0.0.1`
    /// are different strings, so pointing the same server at a different
    /// name re-synthesizes the project once. That is the right trade —
    /// a spurious miss is slow and visible, whereas a spurious hit serves
    /// one voice's audio under another's name, permanently.
    pub fn version_string(&self) -> String {
        let host = self.base_url.split("://").nth(1).unwrap_or(&self.base_url);
        format!("{}@{}", self.model, host)
    }
}
