use teleprompt_voice::{
    async_trait, LanguageSupport, SynthRequest, Synthesized, VoiceBackend, VoiceCapabilities,
    VoiceError,
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

    /// The voices the server reports. Used by `doctor` and by `dub`'s
    /// one-shot validation — never by `check`, which must not touch the
    /// network.
    pub async fn voices(&self) -> Result<Vec<String>, VoiceError> {
        self.client.voices().await
    }

    pub fn base_url(&self) -> &str {
        &self.client.config().base_url
    }

    pub fn concurrency(&self) -> usize {
        self.client.config().concurrency
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
            // Available only on POST /dev/captioned_speech. A `/dev/` path
            // is not a stable interface to build a published manifest field
            // on. Revisit when it stabilises; the manifest already omits
            // `words` when absent.
            word_timings: false,
            ssml: false,
            speed_control: true,
            version: self.version.clone(),
        }
    }

    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        let pcm = self
            .client
            .speech(&req.text, req.voice.as_deref(), req.speed)
            .await?;
        Ok(Synthesized {
            pcm,
            word_timings: None,
        })
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
