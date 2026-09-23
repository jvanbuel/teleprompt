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

    /// What `VoiceCapabilities::version` returns, and therefore part of
    /// every cache key this backend's audio is stored under: the model, and
    /// nothing else.
    ///
    /// Not the server's address. Audio depends on the weights a server
    /// loaded, not on where that server is, and keying on the address means
    /// a cache that never crosses a machine: CI starts cold, a second
    /// contributor starts cold, and on one machine `localhost` and
    /// `127.0.0.1` are two caches for one server. A key that travels is
    /// what makes a shared cache possible at all.
    ///
    /// The trade is real and deliberate, and it runs the other way from the
    /// one this used to make: two servers serving different weights under
    /// one model name now collide, and the cache cannot tell them apart.
    /// `doctor` reports the model beside the address so the mismatch is
    /// visible where somebody is already looking, and an author running two
    /// sets of weights distinguishes them by naming them — `model:
    /// kokoro-v1_1` — which is the field that exists for that.
    ///
    /// With `word_timings` on, `+words` is added: the audio is the same,
    /// but an entry cached without timings would keep answering without
    /// them, so turning them on re-synthesizes each line once.
    pub fn version_string(&self) -> String {
        if self.word_timings {
            format!("{}+words", self.model)
        } else {
            self.model.clone()
        }
    }
}
