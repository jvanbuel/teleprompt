use std::sync::Arc;

use teleprompt_voice::{
    async_trait, Provider, SynthRequest, Synthesized, VoiceBackend, VoiceError,
};

use crate::client::Client;
use crate::config::GeminiConfig;

/// The voice spoken in when `voice.voice` names none: one of the thirty
/// prebuilt ones, firm and clear.
pub const DEFAULT_VOICE: &str = "Kore";

pub struct GeminiVoice {
    client: Client,
    version: String,
}

impl GeminiVoice {
    pub fn new(cfg: GeminiConfig) -> Result<Self, String> {
        let version = cfg.version_string();
        Ok(Self {
            client: Client::new(cfg)?,
            version,
        })
    }

    /// For `setup`: the address, and whether the key opens the model.
    pub async fn check(&self) -> Result<String, VoiceError> {
        let name = self.client.model_name().await?;
        Ok(format!(
            "{} — key accepted, {name} ({})",
            self.client.config().base_url,
            self.version
        ))
    }
}

/// Gemini as teleprompt registers it: built from `[backends.gemini]`.
pub fn provider() -> Provider {
    Provider {
        id: "gemini",
        build: |settings| {
            let cfg = match settings {
                Some(v) => GeminiConfig::from_value(v)?,
                None => GeminiConfig::default(),
            };
            Ok(Arc::new(GeminiVoice::new(cfg)?))
        },
    }
}

#[async_trait]
impl VoiceBackend for GeminiVoice {
    fn id(&self) -> &str {
        "gemini"
    }

    fn version(&self) -> String {
        self.version.clone()
    }

    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        if req.speed != 1.0 {
            return Err(self.client.fail(&format!(
                "takes no speed, and `voice.speed` is {}: set it to 1.0, \
                 or ask for a pace with `voice.instruct`, such as \"a little slower\"",
                req.speed
            )));
        }
        let voice = req.voice.as_deref().unwrap_or(DEFAULT_VOICE);
        let pcm = self
            .client
            .speak(&req.text, voice, req.instruct.as_deref())
            .await?;
        Ok(Synthesized {
            pcm,
            word_timings: None,
        })
    }

    fn concurrency(&self) -> usize {
        self.client.config().concurrency
    }

    fn address(&self) -> Option<String> {
        Some(self.client.config().base_url.clone())
    }

    async fn probe(&self) -> Result<String, VoiceError> {
        self.check().await
    }
}
