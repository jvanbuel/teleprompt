use serde::{Deserialize, Serialize};

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
    /// The backend's own version, such as its model; part of the cache key
    /// (`docs/design.md#voice-cache`).
    ///
    /// **The only handle a backend has on its cache key**, which otherwise
    /// covers just the id and the [`SynthRequest`]. Configuration that
    /// changes the audio but is not in the request (a model checkpoint, a
    /// sample rate, a vocoder setting) **must** be folded in here, or two
    /// differently configured instances silently serve each other's audio
    /// as `measured`. Where the server is does not change the audio, so the
    /// address stays out.
    pub version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SynthRequest {
    pub text: String,
    pub locale: String,
    pub voice: Option<String>,
    pub speed: f64,
    /// How to deliver the line, for a backend that takes instructions
    /// ("warmly, with a smile"); others ignore it.
    pub instruct: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordTiming {
    pub word: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// What a backend produces. There is no separate duration: it is
/// [`Pcm::duration_ms`] (`docs/design.md#voice-contract`).
#[derive(Debug, Clone)]
pub struct Synthesized {
    pub pcm: Pcm,
    /// Present only when `capabilities().word_timings` is true.
    pub word_timings: Option<Vec<WordTiming>>,
}

/// Interleaved 16-bit PCM, the only audio that crosses the backend
/// boundary. Encoding to a file format happens downstream, in one place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    pub sample_rate: u32,
    pub channels: u16,
    pub samples: Vec<i16>,
}

impl Pcm {
    /// Computed per *frame*: with two channels, two samples are one instant.
    pub fn duration_ms(&self) -> u64 {
        let channels = self.channels.max(1) as u64;
        let rate = self.sample_rate.max(1) as u64;
        let frames = self.samples.len() as u64 / channels;
        frames * 1000 / rate
    }

    /// The same audio at `rate`, exactly as many milliseconds long: the
    /// frame count is rounded up to the length's, and never by a whole
    /// millisecond.
    pub fn resampled(&self, rate: u32) -> Pcm {
        let channels = self.channels.max(1) as usize;
        let frames = (self.duration_ms() * rate as u64).div_ceil(1000) as usize;
        let per_channel: Vec<Vec<f32>> = (0..channels)
            .map(|c| {
                let mono: Vec<f32> = self
                    .samples
                    .iter()
                    .skip(c)
                    .step_by(channels)
                    .map(|&s| s as f32 / 32768.0)
                    .collect();
                let mut out = super::resample(&mono, self.sample_rate, rate);
                out.resize(frames, 0.0);
                out
            })
            .collect();
        let samples = (0..frames)
            .flat_map(|f| per_channel.iter().map(move |ch| ch[f]))
            .map(|s| {
                (s * 32768.0)
                    .round()
                    .clamp(i16::MIN as f32, i16::MAX as f32) as i16
            })
            .collect();
        Pcm {
            sample_rate: rate,
            channels: self.channels,
            samples,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    /// `backend` is owned because an id need not be a literal (one crate can
    /// serve several configured endpoints).
    #[error("backend `{backend}` does not support {what}")]
    Unsupported { backend: String, what: String },
    #[error("{0}")]
    Other(String),
}

/// A recording of the author to clone a voice from, and what it says.
#[derive(Debug, Clone)]
pub struct VoiceSample {
    /// The file name the server is given.
    pub file: String,
    pub wav: Vec<u8>,
    pub text: String,
}

/// A voice a backend made: its name, as `voice.voice` gives it, and its
/// server's own id for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClonedVoice {
    pub name: String,
    pub id: String,
}

/// See `docs/design.md#voice-contract`. There is deliberately no downcast:
/// what teleprompt asks of a backend beyond speaking a line is a method
/// here, with a default for a backend that has nothing to say, so the CLI
/// holds every backend the same way.
#[async_trait::async_trait]
pub trait VoiceBackend: Send + Sync {
    fn id(&self) -> &str;
    fn capabilities(&self) -> VoiceCapabilities;

    /// Caching and duration prediction are not the backend's concern.
    async fn synthesize(&self, req: &SynthRequest) -> Result<Synthesized, VoiceError>;

    /// How many lines `dub` sends it at once.
    fn concurrency(&self) -> usize {
        1
    }

    /// Where its server is, as `setup` and errors name it; `None` for a
    /// backend with no server.
    fn address(&self) -> Option<String> {
        None
    }

    /// The voices its server offers, which `dub` checks a script's against
    /// before synthesizing anything; `None` when it cannot list them.
    /// Never called by `check`, which does not touch the network.
    async fn voices(&self) -> Option<Result<Vec<String>, VoiceError>> {
        None
    }

    /// One line on its server for `setup`: that it answers, and what with
    /// (a model, its voices, whether a key is accepted).
    async fn probe(&self) -> Result<String, VoiceError> {
        Err(self.unsupported("a probe"))
    }

    /// A voice named `name`, cloned from `samples` of the author's takes,
    /// where `capabilities().cloning`.
    async fn clone_voice(
        &self,
        _name: &str,
        _language: &str,
        _samples: &[VoiceSample],
    ) -> Result<ClonedVoice, VoiceError> {
        Err(self.unsupported("cloning a voice"))
    }

    /// The error a default method answers with.
    fn unsupported(&self, what: &str) -> VoiceError {
        VoiceError::Unsupported {
            backend: self.id().to_string(),
            what: what.to_string(),
        }
    }
}
