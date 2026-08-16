//! A minimal `VoiceBackend` for exercising `VoiceRegistry` without pulling
//! in `teleprompt-voice-null`. The registry only ever looks at `id()`, so
//! everything else here is the smallest thing that satisfies the trait.

use teleprompt_voice::async_trait;
use teleprompt_voice::{
    LanguageSupport, SynthRequest, Synthesized, VoiceBackend, VoiceCapabilities, VoiceError,
};

pub struct StubVoice {
    id: &'static str,
}

impl StubVoice {
    pub fn new(id: &'static str) -> Self {
        Self { id }
    }
}

#[async_trait]
impl VoiceBackend for StubVoice {
    fn id(&self) -> &str {
        self.id
    }

    fn capabilities(&self) -> VoiceCapabilities {
        VoiceCapabilities {
            languages: LanguageSupport::Any,
            cloning: false,
            cross_lingual: false,
            word_timings: false,
            ssml: false,
            speed_control: false,
            version: "0.0.0-stub".to_string(),
        }
    }

    async fn synthesize(&self, _req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        Err(VoiceError::Other("stub backend cannot synthesize".into()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
