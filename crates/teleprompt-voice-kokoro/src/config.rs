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
    /// Ask for per-word timings, which make `cue=` exact and fill the
    /// manifest's `words`. They come from Kokoro-FastAPI's
    /// `/dev/captioned_speech`, a `/dev/` path and so not a stable
    /// interface, which is why this is opt-in: a server without it fails
    /// the dub, naming this setting.
    pub word_timings: bool,
}

impl Default for KokoroConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:8880".to_string(),
            timeout_ms: 30_000,
            concurrency: 4,
            model: "kokoro".to_string(),
            word_timings: false,
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

    /// What `VoiceCapabilities::version` returns, and so the cache key for
    /// this backend's audio: the model, and not the server's address, so a
    /// cache travels between machines (docs/design.md#voice-cache). Two
    /// servers with different weights under one model name collide; name
    /// them apart (`model = "kokoro-v1_1"`).
    ///
    /// `+words` is added with `word_timings` on, so an entry cached without
    /// timings never answers for one that needs them.
    pub fn version_string(&self) -> String {
        if self.word_timings {
            format!("{}+words", self.model)
        } else {
            self.model.clone()
        }
    }
}
