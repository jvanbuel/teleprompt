//! A minimal `VoiceBackend` for exercising `VoiceRegistry` without pulling
//! in `teleprompt-voice-null`. The registry only ever looks at `id()`, so
//! everything else here is the smallest thing that satisfies the trait.
//!
//! Still synchronous, matching `VoiceBackend` as it stands at this task.
//! Task 6 converts both the trait and this stub to async and names this
//! file when it does.

use teleprompt_voice::{
    LanguageSupport, Pcm, SynthRequest, SynthResult, VoiceBackend, VoiceCapabilities, VoiceError,
};

pub struct StubVoice {
    id: &'static str,
}

impl StubVoice {
    pub fn new(id: &'static str) -> Self {
        Self { id }
    }
}

impl VoiceBackend for StubVoice {
    fn id(&self) -> &'static str {
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

    fn synthesize(&self, _req: &SynthRequest) -> Result<SynthResult, VoiceError> {
        Err(VoiceError::Other("stub backend cannot synthesize".into()))
    }

    fn render_pcm(&self, _req: &SynthRequest) -> Result<Option<Pcm>, VoiceError> {
        Err(VoiceError::Other("stub backend cannot render".into()))
    }

    fn cache_key(&self, _req: &SynthRequest) -> String {
        format!("stub/{}", self.id)
    }
}
