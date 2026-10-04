use teleprompt_plugin::voice::{
    async_trait, LanguageSupport, SynthRequest, Synthesized, VoiceBackend, VoiceCapabilities,
    VoiceError,
};

use crate::client::{Client, Profile, Sample};
use crate::config::VoiceboxConfig;

pub struct VoiceboxVoice {
    client: Client,
    version: String,
}

impl VoiceboxVoice {
    pub fn new(cfg: VoiceboxConfig) -> Result<Self, String> {
        let version = cfg.version_string();
        Ok(Self {
            client: Client::new(cfg)?,
            version,
        })
    }

    /// The voices on the server, for `setup` and `voice clone`; never for
    /// `check`, which does not touch the network.
    pub async fn profiles(&self) -> Result<Vec<Profile>, VoiceError> {
        self.client.profiles().await
    }

    /// A voice named `name`, cloned from `samples` (docs/design.md#voice).
    pub async fn clone_voice(
        &self,
        name: &str,
        language: &str,
        samples: &[Sample],
    ) -> Result<Profile, VoiceError> {
        self.client.clone_voice(name, language, samples).await
    }

    pub fn base_url(&self) -> &str {
        &self.client.config().base_url
    }

    pub fn concurrency(&self) -> usize {
        self.client.config().concurrency
    }
}

#[async_trait]
impl VoiceBackend for VoiceboxVoice {
    fn id(&self) -> &str {
        "voicebox"
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
        let Some(voice) = req.voice.as_deref() else {
            return Err(self.client.fail(
                "needs a voice: set `voice.voice` to a Voicebox profile; \
                 `teleprompt voice clone <name>` makes one from your takes",
            ));
        };
        if req.speed != 1.0 {
            return Err(self.client.fail(&format!(
                "takes no speed, and `voice.speed` is {}: set it to 1.0, \
                 or ask for a pace with `voice.instruct`, such as \"a little slower\"",
                req.speed
            )));
        }
        let id = self.client.profile_id(voice).await?;
        // Voicebox takes the language alone: `en`, not `en-GB`.
        let language = req.locale.split(['-', '_']).next().unwrap_or("en");
        let pcm = self
            .client
            .speak(&id, &req.text, language, req.instruct.as_deref())
            .await?;
        Ok(Synthesized {
            pcm,
            word_timings: None,
        })
    }
}
