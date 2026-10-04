//! A minimal `VoiceBackend` for exercising `VoiceRegistry` without pulling
//! in `teleprompt_voice::null`. The registry only ever looks at `id()`, so
//! everything else here is the smallest thing that satisfies the trait.

use teleprompt_voice::async_trait;
use teleprompt_voice::{SynthRequest, Synthesized, VoiceBackend, VoiceError};

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

    fn version(&self) -> String {
        "0.0.0-stub".to_string()
    }

    async fn synthesize(&self, _req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        Err(VoiceError::Other("stub backend cannot synthesize".into()))
    }
}
