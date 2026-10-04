//! The reference backend: silence of exactly the estimated length, for
//! `check`, `plan` and tests, and every project until it picks a voice.

use teleprompt_plugin::voice::async_trait;
use teleprompt_plugin::voice::{
    LanguageSupport, Pcm, SynthRequest, Synthesized, VoiceBackend, VoiceCapabilities, VoiceError,
};

use crate::estimator::estimate_ms;
use crate::DEFAULT_WPM;

pub struct NullVoice {
    pub wpm: f64,
}

impl Default for NullVoice {
    fn default() -> Self {
        Self { wpm: DEFAULT_WPM }
    }
}

/// 48 kHz because it divides a millisecond exactly (48 samples), so a
/// duration in ms is always a whole number of frames and silence is never
/// a rounding away from the estimate it is supposed to match.
pub const NULL_SAMPLE_RATE: u32 = 48_000;

#[async_trait]
impl VoiceBackend for NullVoice {
    fn id(&self) -> &str {
        "null"
    }

    fn capabilities(&self) -> VoiceCapabilities {
        VoiceCapabilities {
            languages: LanguageSupport::Any,
            cloning: false,
            cross_lingual: false,
            word_timings: false,
            ssml: false,
            speed_control: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError> {
        if req.speed <= 0.0 {
            return Err(VoiceError::Other("speed must be greater than zero".into()));
        }
        let ms = estimate_ms(&req.text, self.wpm, req.speed);
        let frames = (ms * NULL_SAMPLE_RATE as u64).div_ceil(1000) as usize;
        Ok(Synthesized {
            pcm: Pcm {
                sample_rate: NULL_SAMPLE_RATE,
                channels: 1,
                samples: vec![0; frames],
            },
            word_timings: None,
        })
    }
}
