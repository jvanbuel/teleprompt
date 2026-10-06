use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct VoiceboxConfig {
    pub base_url: String,
    /// A cloned voice on a large model takes a while per line.
    pub timeout_ms: u64,
    /// Lines `dub` sends at once. One model on one machine: more only
    /// queues.
    pub concurrency: usize,
    /// Which of Voicebox's engines speaks: `qwen` (the default, for cloned
    /// and designed voices), `chatterbox`, `luxtts`, `kokoro`, ….
    pub engine: String,
    /// The engine's model size, such as `1.7B` or `0.6B`; the server's own
    /// default when unset.
    pub model_size: Option<String>,
    /// Sent with every line, so the same line sounds the same each time it
    /// is synthesized, as a cache demands.
    pub seed: u64,
}

impl Default for VoiceboxConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:17493".to_string(),
            timeout_ms: 300_000,
            concurrency: 1,
            engine: "qwen".to_string(),
            model_size: None,
            seed: 0,
        }
    }
}

impl VoiceboxConfig {
    pub fn from_value(v: &serde_yaml::Value) -> Result<Self, String> {
        let mut c: VoiceboxConfig = serde_yaml::from_value(v.clone())
            .map_err(|e| format!("invalid `backends.voicebox` settings: {e}"))?;
        while c.base_url.ends_with('/') {
            c.base_url.pop();
        }
        if c.base_url.is_empty() {
            return Err("`backends.voicebox.base_url` must not be empty".to_string());
        }
        if c.concurrency == 0 {
            return Err("`backends.voicebox.concurrency` must be at least 1".to_string());
        }
        Ok(c)
    }

    /// The cache key's handle on what changes the audio beyond the request
    /// (docs/design.md#voice-cache): the engine, its size and the seed. The
    /// profile is the request's `voice`; a profile cloned again under the
    /// same name keeps its old audio until the cache is cleared.
    pub fn version_string(&self) -> String {
        format!(
            "voicebox/{}/{}/seed{}",
            self.engine,
            self.model_size.as_deref().unwrap_or("default"),
            self.seed
        )
    }
}
