use std::sync::Arc;

use teleprompt_plugin::voice::{
    async_trait, LanguageSupport, SynthRequest, Synthesized, VoiceBackend, VoiceCapabilities,
    VoiceError, VoicePlugin,
};

use crate::client::Client;
use crate::config::KokoroConfig;

pub struct KokoroVoice {
    client: Client,
    version: String,
}

impl KokoroVoice {
    pub fn new(cfg: KokoroConfig) -> Result<Self, String> {
        let version = cfg.version_string();
        Ok(Self {
            client: Client::new(cfg)?,
            version,
        })
    }

    /// The voices the server reports. Used by `setup` and by `dub`'s
    /// one-shot validation — never by `check`, which must not touch the
    /// network.
    pub async fn voices(&self) -> Result<Vec<String>, VoiceError> {
        self.client.voices().await
    }
}

/// What it needs that teleprompt does not ship.
static NEEDS: [&teleprompt_plugin::tool::Tool; 1] = [&crate::tools::KOKORO];

/// Kokoro as teleprompt registers it: built from `[backends.kokoro]`.
pub fn plugin() -> VoicePlugin {
    VoicePlugin {
        id: "kokoro",
        needs: &NEEDS,
        build: |settings| {
            let cfg = match settings {
                Some(v) => KokoroConfig::from_value(v)?,
                None => KokoroConfig::default(),
            };
            Ok(Arc::new(KokoroVoice::new(cfg)?))
        },
    }
}

#[async_trait]
impl VoiceBackend for KokoroVoice {
    fn id(&self) -> &str {
        "kokoro"
    }

    fn capabilities(&self) -> VoiceCapabilities {
        VoiceCapabilities {
            // Kokoro's voices are language-specific and the server is the
            // authority on which exist; enumerating a fixed list here would
            // go stale against a server we do not ship.
            languages: LanguageSupport::Any,
            cloning: false,
            cross_lingual: false,
            // Only on POST /dev/captioned_speech, which is a `/dev/` path
            // and so not a stable interface: opt-in, per project.
            word_timings: self.client.config().word_timings,
            ssml: false,
            speed_control: true,
            version: self.version.clone(),
        }
    }

    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        if self.client.config().word_timings {
            let (pcm, words) = self
                .client
                .captioned(&req.text, req.voice.as_deref(), req.speed)
                .await?;
            return Ok(Synthesized {
                pcm,
                word_timings: Some(words),
            });
        }
        let pcm = self
            .client
            .speech(&req.text, req.voice.as_deref(), req.speed)
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

    async fn voices(&self) -> Option<Result<Vec<String>, VoiceError>> {
        Some(self.client.voices().await)
    }

    /// The model beside the address: docs/design.md#voice-cache.
    async fn probe(&self) -> Result<String, VoiceError> {
        let voices = self.client.voices().await?;
        Ok(format!(
            "{} — reachable, model {}, {} voices",
            self.client.config().base_url,
            self.version,
            voices.len()
        ))
    }
}
