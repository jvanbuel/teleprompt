use std::sync::Arc;

use teleprompt_voice::{
    async_trait, Provider, SynthRequest, Synthesized, VoiceBackend, VoiceError,
};

use crate::client::Client;
use crate::config::{OpenAiConfig, Preset};

/// A server that speaks OpenAI's speech API: Kokoro, OpenAI itself, or any
/// other under a name of the author's.
pub struct OpenAiVoice {
    client: Client,
    version: String,
}

impl OpenAiVoice {
    pub fn new(cfg: OpenAiConfig) -> Result<Self, String> {
        let version = cfg.version_string();
        Ok(Self {
            client: Client::new(cfg)?,
            version,
        })
    }

    pub fn config(&self) -> &OpenAiConfig {
        self.client.config()
    }
}

/// `defaults`, with the author's settings over them.
fn build(
    defaults: OpenAiConfig,
    settings: Option<&serde_yaml::Value>,
) -> Result<Arc<dyn VoiceBackend>, String> {
    let cfg = match settings {
        Some(v) => defaults.with(v)?,
        None => defaults,
    };
    Ok(Arc::new(OpenAiVoice::new(cfg)?))
}

/// Kokoro-FastAPI, built from `[backends.kokoro]`.
pub fn kokoro() -> Provider {
    Provider {
        id: "kokoro",
        build: |settings| build(OpenAiConfig::kokoro(), settings),
    }
}

/// OpenAI's service, built from `[backends.openai]`.
pub fn openai() -> Provider {
    Provider {
        id: "openai",
        build: |settings| build(OpenAiConfig::openai(), settings),
    }
}

/// A server of the author's: any `[backends.<name>]` that names no voice
/// teleprompt ships.
pub fn endpoint(name: &str, settings: &serde_yaml::Value) -> Result<Arc<dyn VoiceBackend>, String> {
    build(OpenAiConfig::endpoint(name), Some(settings))
}

#[async_trait]
impl VoiceBackend for OpenAiVoice {
    fn id(&self) -> &str {
        &self.config().id
    }

    fn version(&self) -> String {
        self.version.clone()
    }

    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        let voice = req.voice.as_deref();
        let instruct = req.instruct.as_deref();
        if self.config().word_timings {
            let (pcm, words) = self
                .client
                .captioned(&req.text, voice, req.speed, instruct)
                .await?;
            return Ok(Synthesized {
                pcm,
                word_timings: Some(words),
            });
        }
        let pcm = self
            .client
            .speech(&req.text, voice, req.speed, instruct)
            .await?;
        Ok(Synthesized {
            pcm,
            word_timings: None,
        })
    }

    fn concurrency(&self) -> usize {
        self.config().concurrency
    }

    fn address(&self) -> Option<String> {
        Some(self.config().base_url.clone())
    }

    /// Used by `setup` and by `dub`'s one-shot validation — never by
    /// `check`, which must not touch the network.
    async fn voices(&self) -> Option<Result<Vec<String>, VoiceError>> {
        self.client.voices().await.transpose()
    }

    /// The model beside the address: docs/design.md#voice-cache.
    async fn probe(&self) -> Result<String, VoiceError> {
        let cfg = self.config();
        let listed = match self.client.voices().await? {
            Some(voices) => format!(", {} voices", voices.len()),
            None => {
                self.client.reachable().await?;
                String::new()
            }
        };
        let key = match (&cfg.api_key_env, cfg.preset) {
            (Some(var), _) => format!(", key from {var}"),
            (None, Preset::OpenAi) => ", no key".to_string(),
            _ => String::new(),
        };
        Ok(format!(
            "{} — reachable, model {}{listed}{key}",
            cfg.base_url, self.version
        ))
    }
}
