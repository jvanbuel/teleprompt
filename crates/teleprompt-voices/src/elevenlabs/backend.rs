use std::sync::Arc;

use teleprompt_voice::{
    async_trait, Provider, SynthRequest, Synthesized, VoiceBackend, VoiceError,
};

use super::client::Client;
use super::config::ElevenLabsConfig;

/// The voice spoken in when `voice.voice` names none: George, a premade
/// voice every account has, by its id.
pub const DEFAULT_VOICE: &str = "JBFqnCBsd6RMkjVDRZzb";

/// The speeds ElevenLabs takes.
const SPEEDS: std::ops::RangeInclusive<f64> = 0.7..=1.2;

pub struct ElevenLabsVoice {
    client: Client,
    version: String,
}

impl ElevenLabsVoice {
    pub fn new(cfg: ElevenLabsConfig) -> Result<Self, String> {
        let version = cfg.version_string();
        Ok(Self {
            client: Client::new(cfg)?,
            version,
        })
    }

    /// The voice's id: `voice` as given if it is one, or the id of the
    /// voice the account names so.
    async fn voice_id(&self, voice: &str) -> Result<String, VoiceError> {
        let listed = self.client.voices().await?;
        if listed.iter().any(|v| v.id == voice) {
            return Ok(voice.to_string());
        }
        listed
            .iter()
            .find(|v| v.name.eq_ignore_ascii_case(voice))
            .map(|v| v.id.clone())
            .ok_or_else(|| {
                self.client.fail(&format!(
                    "has no voice `{voice}`; `teleprompt setup elevenlabs` lists them"
                ))
            })
    }
}

/// ElevenLabs as teleprompt registers it: built from `[backends.elevenlabs]`.
pub fn provider() -> Provider {
    Provider {
        id: "elevenlabs",
        build: |settings| {
            let cfg = match settings {
                Some(v) => ElevenLabsConfig::from_value(v)?,
                None => ElevenLabsConfig::default(),
            };
            Ok(Arc::new(ElevenLabsVoice::new(cfg)?))
        },
    }
}

#[async_trait]
impl VoiceBackend for ElevenLabsVoice {
    fn id(&self) -> &str {
        "elevenlabs"
    }

    fn version(&self) -> String {
        self.version.clone()
    }

    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        if req.instruct.is_some() {
            return Err(self.client.fail(
                "takes no delivery instructions, and the words are spoken as written: \
                 leave `voice.instruct` unset for this voice",
            ));
        }
        if !SPEEDS.contains(&req.speed) {
            return Err(self.client.fail(&format!(
                "speaks from 0.7 to 1.2 times its pace, and `voice.speed` is {}",
                req.speed
            )));
        }
        let voice = match req.voice.as_deref() {
            Some(voice) => self.voice_id(voice).await?,
            None => DEFAULT_VOICE.to_string(),
        };
        let (pcm, words) = self.client.speak(&req.text, &voice, req.speed).await?;
        Ok(Synthesized {
            pcm,
            word_timings: (!words.is_empty()).then_some(words),
        })
    }

    fn concurrency(&self) -> usize {
        self.client.config().concurrency
    }

    fn address(&self) -> Option<String> {
        Some(self.client.config().base_url.clone())
    }

    /// Each voice by its name and by its id, either of which a script may give.
    async fn voices(&self) -> Option<Result<Vec<String>, VoiceError>> {
        Some(self.client.voices().await.map(|listed| {
            listed
                .into_iter()
                .flat_map(|v| [v.name, v.id])
                .filter(|s| !s.is_empty())
                .collect()
        }))
    }

    /// Whether the key opens the account, and how many voices it has.
    async fn probe(&self) -> Result<String, VoiceError> {
        let listed = self.client.voices().await?;
        Ok(format!(
            "{} — key accepted, {} voices ({})",
            self.client.config().base_url,
            listed.len(),
            self.version
        ))
    }
}
