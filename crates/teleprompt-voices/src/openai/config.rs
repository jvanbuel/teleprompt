use serde::Deserialize;

/// What `response_format: "pcm"` is in OpenAI's speech API, and so what
/// every server that follows it sends unless told otherwise: 16-bit
/// little-endian mono at 24 kHz. `Pcm` carries the rate, so a server that
/// sends another names it with `sample_rate`.
pub const PCM_SAMPLE_RATE: u32 = 24_000;

/// Which server the settings are for, which decides their defaults and
/// what it can do beyond speaking a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    /// Kokoro-FastAPI, run by the author: lists its voices, and times
    /// words on request.
    Kokoro,
    /// OpenAI's own service, with the author's key.
    OpenAi,
    /// Any other server that speaks the API, under a name of the author's
    /// (`[backends.<name>]` with its `base_url`).
    #[default]
    Endpoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct OpenAiConfig {
    /// The id `voice.backend` names it by; not a setting.
    #[serde(skip)]
    pub id: String,
    #[serde(skip)]
    pub preset: Preset,
    /// The API's root, as OpenAI's own SDKs take it (`…/v1`). A bare
    /// server address, with no path, gets `/v1`.
    pub base_url: String,
    /// The environment variable holding the API key, if the server wants
    /// one. Never the key itself: `teleprompt.toml` is committed.
    pub api_key_env: Option<String>,
    pub timeout_ms: u64,
    /// Bounded fan-out for `dub`. A local model server is the bottleneck and
    /// unbounded concurrency makes it slower, not faster.
    pub concurrency: usize,
    /// Sent as the request's `model` field and folded into the cache key.
    pub model: String,
    /// The voice when `voice.voice` names none; the API requires one.
    pub voice: Option<String>,
    /// The rate of the server's PCM, where it is not 24 kHz.
    pub sample_rate: u32,
    /// Ask for per-word timings, which make `cue=` exact and fill the
    /// manifest's `words`. They come from Kokoro-FastAPI's
    /// `/dev/captioned_speech`, a `/dev/` path and so not a stable
    /// interface, which is why this is opt-in: a server without it fails
    /// the dub, naming this setting.
    pub word_timings: bool,
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self::kokoro()
    }
}

impl OpenAiConfig {
    /// Kokoro-FastAPI on this machine, as its README runs it.
    pub fn kokoro() -> Self {
        Self {
            id: "kokoro".to_string(),
            preset: Preset::Kokoro,
            base_url: "http://localhost:8880".to_string(),
            api_key_env: None,
            timeout_ms: 30_000,
            concurrency: 4,
            model: "kokoro".to_string(),
            voice: None,
            sample_rate: PCM_SAMPLE_RATE,
            word_timings: false,
        }
    }

    /// OpenAI's service: its key in `OPENAI_API_KEY`.
    pub fn openai() -> Self {
        Self {
            id: "openai".to_string(),
            preset: Preset::OpenAi,
            base_url: "https://api.openai.com/v1".to_string(),
            api_key_env: Some("OPENAI_API_KEY".to_string()),
            timeout_ms: 60_000,
            model: "gpt-4o-mini-tts".to_string(),
            voice: Some("alloy".to_string()),
            ..Self::kokoro()
        }
    }

    /// A server of the author's, named `id`: its address is theirs to give.
    pub fn endpoint(id: &str) -> Self {
        Self {
            id: id.to_string(),
            preset: Preset::Endpoint,
            base_url: String::new(),
            timeout_ms: 60_000,
            concurrency: 2,
            model: "tts-1".to_string(),
            ..Self::kokoro()
        }
    }

    /// These defaults, with `v`'s settings over them.
    pub fn with(&self, v: &serde_yaml::Value) -> Result<Self, String> {
        let id = self.id.clone();
        let mut merged = serde_yaml::to_value(Shadow::from(self)).expect("plain data");
        if let (Some(base), Some(given)) = (merged.as_mapping_mut(), v.as_mapping()) {
            for (k, val) in given {
                base.insert(k.clone(), val.clone());
            }
        } else if !v.is_null() {
            return Err(format!("`backends.{id}` must be a table of settings"));
        }
        let mut c: OpenAiConfig = serde_yaml::from_value(merged)
            .map_err(|e| format!("invalid `backends.{id}` settings: {e}"))?;
        c.id = id;
        c.preset = self.preset;
        c.check()?;
        Ok(c)
    }

    fn check(&mut self) -> Result<(), String> {
        let id = &self.id;
        while self.base_url.ends_with('/') {
            self.base_url.pop();
        }
        if self.base_url.is_empty() {
            return Err(match self.preset {
                Preset::Endpoint => format!(
                    "`backends.{id}` needs `base_url`: the server's API root, \
                     such as \"http://localhost:8880/v1\""
                ),
                _ => format!("`backends.{id}.base_url` must not be empty"),
            });
        }
        if self.concurrency == 0 {
            return Err(format!("`backends.{id}.concurrency` must be at least 1"));
        }
        if self.sample_rate == 0 {
            return Err(format!("`backends.{id}.sample_rate` must be at least 1"));
        }
        Ok(())
    }

    /// The API's root: `base_url`, with `/v1` where it names a server and
    /// no path.
    pub fn api_root(&self) -> String {
        let path = self
            .base_url
            .split_once("://")
            .map_or("", |(_, rest)| rest.split_once('/').map_or("", |(_, p)| p));
        if path.is_empty() {
            format!("{}/v1", self.base_url)
        } else {
            self.base_url.clone()
        }
    }

    /// The server itself, for paths outside the API (Kokoro's `/dev/`).
    pub fn server_root(&self) -> String {
        let root = self.api_root();
        root.strip_suffix("/v1").unwrap_or(&root).to_string()
    }

    /// What [`teleprompt_voice::VoiceBackend::version`] returns, and so the cache key for
    /// this backend's audio: the model, and not the server's address, so a
    /// cache travels between machines (docs/design.md#voice-cache). Two
    /// servers with different weights under one model name collide; name
    /// them apart (`model = "kokoro-v1_1"`).
    ///
    /// `+words` is added with `word_timings` on, so an entry cached without
    /// timings never answers for one that needs them; the rate, where it is
    /// not the API's own.
    pub fn version_string(&self) -> String {
        let mut v = self.model.clone();
        if self.word_timings {
            v.push_str("+words");
        }
        if self.sample_rate != PCM_SAMPLE_RATE {
            v.push_str(&format!("@{}", self.sample_rate));
        }
        v
    }
}

/// The settings alone, for merging the author's over a preset's.
#[derive(serde::Serialize)]
struct Shadow<'a> {
    base_url: &'a str,
    api_key_env: &'a Option<String>,
    timeout_ms: u64,
    concurrency: usize,
    model: &'a str,
    voice: &'a Option<String>,
    sample_rate: u32,
    word_timings: bool,
}

impl<'a> From<&'a OpenAiConfig> for Shadow<'a> {
    fn from(c: &'a OpenAiConfig) -> Self {
        Shadow {
            base_url: &c.base_url,
            api_key_env: &c.api_key_env,
            timeout_ms: c.timeout_ms,
            concurrency: c.concurrency,
            model: &c.model,
            voice: &c.voice,
            sample_rate: c.sample_rate,
            word_timings: c.word_timings,
        }
    }
}
