use std::sync::Arc;

use teleprompt_voice::{
    async_trait, ClonedVoice, Provider, SynthRequest, Synthesized, VoiceBackend, VoiceError,
    VoiceSample,
};

use crate::client::{Client, Profile};
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
}

/// Voicebox as teleprompt registers it: built from `[backends.voicebox]`.
pub fn provider() -> Provider {
    Provider {
        id: "voicebox",
        build: |settings| {
            let cfg = match settings {
                Some(v) => VoiceboxConfig::from_value(v)?,
                None => VoiceboxConfig::default(),
            };
            Ok(Arc::new(VoiceboxVoice::new(cfg)?))
        },
    }
}

#[async_trait]
impl VoiceBackend for VoiceboxVoice {
    fn id(&self) -> &str {
        "voicebox"
    }

    fn version(&self) -> String {
        self.version.clone()
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

    fn concurrency(&self) -> usize {
        self.client.config().concurrency
    }

    fn address(&self) -> Option<String> {
        Some(self.client.config().base_url.clone())
    }

    async fn probe(&self) -> Result<String, VoiceError> {
        let profiles = self.client.profiles().await?;
        let voices = if profiles.is_empty() {
            "none yet (`teleprompt voice clone`)".to_string()
        } else {
            let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
            names.join(", ")
        };
        Ok(format!(
            "{} — reachable, {}, voices: {voices}",
            self.client.config().base_url,
            self.version
        ))
    }

    /// A voice named `name`, cloned from `samples` (docs/design.md#voice).
    async fn clone_voice(
        &self,
        name: &str,
        language: &str,
        samples: &[VoiceSample],
    ) -> Result<ClonedVoice, VoiceError> {
        let profile = self.client.clone_voice(name, language, samples).await?;
        Ok(ClonedVoice {
            name: profile.name,
            id: profile.id,
        })
    }
}
