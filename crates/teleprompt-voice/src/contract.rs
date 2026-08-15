use serde::{Deserialize, Serialize};
use teleprompt_core::Hash;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanguageSupport {
    Any,
    Enumerated(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceCapabilities {
    pub languages: LanguageSupport,
    pub cloning: bool,
    pub cross_lingual: bool,
    pub word_timings: bool,
    pub ssml: bool,
    pub speed_control: bool,
    /// The backend's own version, not teleprompt's. Feeds
    /// `teleprompt_cache::key`'s `backend_version`, which is what makes a
    /// backend release turn over its cache entries instead of teleprompt's
    /// own version doing that job for every backend at once.
    pub version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SynthRequest {
    pub text: String,
    pub locale: String,
    pub voice: Option<String>,
    pub speed: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordTiming {
    pub word: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone)]
pub struct SynthResult {
    pub duration_ms: u64,
    pub audio_hash: Hash,
    pub word_timings: Option<Vec<WordTiming>>,
}

/// Interleaved 16-bit PCM. The one audio representation that crosses a
/// backend boundary — encoders live downstream of this type, not inside
/// backends, so every backend produces the same thing and only one place
/// knows about file formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl Pcm {
    /// Length in milliseconds, computed per *frame*: with two channels,
    /// two samples are one instant in time, not two.
    pub fn duration_ms(&self) -> u64 {
        let channels = self.channels.max(1) as u64;
        let rate = self.sample_rate.max(1) as u64;
        let frames = self.samples.len() as u64 / channels;
        frames * 1000 / rate
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("backend `{backend}` does not support {what}")]
    Unsupported { backend: &'static str, what: String },
    #[error("{0}")]
    Other(String),
}

pub trait VoiceBackend: Send + Sync {
    fn id(&self) -> &'static str;
    fn capabilities(&self) -> VoiceCapabilities;
    fn synthesize(&self, req: &SynthRequest) -> Result<SynthResult, VoiceError>;

    /// Render this request to audio. Separate from [`VoiceBackend::synthesize`]
    /// because the inner loop (`plan`, `diff`) needs durations thousands of
    /// times and audio never; making one call do both would put a
    /// multi-megabyte allocation on the hot path.
    ///
    /// Returns `Ok(None)` from a backend that genuinely cannot produce audio.
    /// `null` is not such a backend — it returns silence.
    fn render_pcm(&self, req: &SynthRequest) -> Result<Option<Pcm>, VoiceError>;

    fn cache_key(&self, req: &SynthRequest) -> String;
}
