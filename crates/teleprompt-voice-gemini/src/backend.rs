use teleprompt_voice::{
    async_trait, LanguageSupport, SynthRequest, Synthesized, VoiceBackend, VoiceCapabilities,
    VoiceError,
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

    pub fn concurrency(&self) -> usize {
        self.client.config().concurrency
    }

    /// For `doctor`: the address, and whether the key opens the model.
    pub async fn check(&self) -> Result<String, VoiceError> {
        let name = self.client.model_name().await?;
        Ok(format!(
            "{} — key accepted, {name} ({})",
            self.client.config().base_url,
            self.version
        ))
    }
}

#[async_trait]
impl VoiceBackend for GeminiVoice {
    fn id(&self) -> &str {
        "gemini"
    }

    fn capabilities(&self) -> VoiceCapabilities {
        VoiceCapabilities {
            languages: LanguageSupport::Any,
            cloning: true,
            cross_lingual: true,
            word_timings: false,
            ssml: false,
            speed_control: false,
            version: self.version.clone(),
        }
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
}
