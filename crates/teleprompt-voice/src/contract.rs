use serde::{Deserialize, Serialize};

/// What teleprompt asks a voice for: OpenAI's speech request
/// (`POST /v1/audio/speech`), as much of it as a line needs. `text` is its
/// `input`, `instruct` its `instructions`; the model is the voice's own
/// setting. `locale` is teleprompt's, for a provider that wants a language.
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
    /// Where the voice times its words; spread over the line otherwise.
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
    /// Raw little-endian 16-bit samples, as a speech API sends them. An
    /// empty body, or an odd byte count, is not audio: guessing would
    /// publish a silent or garbled line as a real one.
    pub fn from_le_bytes(bytes: &[u8], sample_rate: u32, channels: u16) -> Result<Pcm, String> {
        if bytes.is_empty() {
            return Err("returned an empty audio body".to_string());
        }
        if !bytes.len().is_multiple_of(2) {
            return Err(format!(
                "returned {} bytes, an odd count for 16-bit samples",
                bytes.len()
            ));
        }
        let samples = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&p| i16::from_le_bytes(p))
            .collect();
        Ok(Pcm {
            sample_rate,
            channels,
            samples,
        })
    }

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

/// A voice: a server that speaks OpenAI's speech API, or a provider
/// teleprompt translates that request for (`docs/design.md#voice-contract`).
/// What it can do beyond speaking a line is a method here, with a default
/// for a voice that cannot, so every voice is held the same way.
#[async_trait::async_trait]
pub trait VoiceBackend: Send + Sync {
    fn id(&self) -> &str;

    /// What its audio depends on beyond the request, such as its model:
    /// part of the cache key (`docs/design.md#voice-cache`). Configuration
    /// that changes the audio but is not in the request **must** be in it,
    /// or two differently configured voices serve each other's audio.
    /// Where the server is does not change the audio, so stays out.
    fn version(&self) -> String;

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
    /// for a provider that clones voices.
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
