use teleprompt_core::Hash;

use crate::contract::{
    LanguageSupport, Pcm, SynthRequest, SynthResult, VoiceBackend, VoiceCapabilities, VoiceError,
};

pub struct NullVoice {
    pub wpm: f64,
}

impl Default for NullVoice {
    fn default() -> Self {
        Self { wpm: 150.0 }
    }
}

const COMMA_MS: u64 = 150;
const CLAUSE_MS: u64 = 250;
const SENTENCE_MS: u64 = 350;

/// 48 kHz because it divides a millisecond exactly (48 samples), so a
/// duration in ms is always a whole number of frames and silence is never
/// a rounding away from the estimate it is supposed to match.
pub const NULL_SAMPLE_RATE: u32 = 48_000;

pub fn estimate_ms(text: &str, wpm: f64, speed: f64) -> u64 {
    let words = text
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .count() as f64;
    if words == 0.0 {
        return 0;
    }
    let speech = words / wpm * 60_000.0;
    let pauses: u64 = text
        .chars()
        .map(|c| match c {
            ',' => COMMA_MS,
            ':' | ';' => CLAUSE_MS,
            '.' | '!' | '?' => SENTENCE_MS,
            _ => 0,
        })
        .sum();
    ((speech + pauses as f64) / speed).round() as u64
}

impl VoiceBackend for NullVoice {
    fn id(&self) -> &'static str {
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
        }
    }

    fn synthesize(&self, req: &SynthRequest) -> Result<SynthResult, VoiceError> {
        if req.speed <= 0.0 {
            return Err(VoiceError::Other("speed must be greater than zero".into()));
        }
        Ok(SynthResult {
            duration_ms: estimate_ms(&req.text, self.wpm, req.speed),
            audio_hash: Hash::of(self.cache_key(req).as_bytes()),
            word_timings: None,
        })
    }

    fn render_pcm(&self, req: &SynthRequest) -> Result<Option<Pcm>, VoiceError> {
        let ms = self.synthesize(req)?.duration_ms;
        let frames = (ms * NULL_SAMPLE_RATE as u64).div_ceil(1000) as usize;
        Ok(Some(Pcm {
            sample_rate: NULL_SAMPLE_RATE,
            channels: 1,
            samples: vec![0; frames],
        }))
    }

    fn cache_key(&self, req: &SynthRequest) -> String {
        format!(
            "null/{}/{}/{}/{}/{}/{}",
            env!("CARGO_PKG_VERSION"),
            req.locale,
            req.voice.as_deref().unwrap_or("-"),
            req.speed,
            self.wpm,
            Hash::of(req.text.as_bytes())
        )
    }
}
