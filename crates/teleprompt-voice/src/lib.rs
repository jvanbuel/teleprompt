//! How a line is spoken. Every voice takes the same request, OpenAI's
//! speech request ([`VoiceBackend`]), and hands back [`Pcm`]. Here too:
//! what speaks a line when nothing else does (`null`), how long a line is
//! expected to take before it is spoken, stretching a line to fit, and the
//! author's own takes.

pub use async_trait::async_trait;
/// Re-exported so a provider uses the same version as the errors here.
pub use teleprompt_core::error::with_causes;

mod contract;
mod resample;
pub mod wav;

pub use contract::{
    ClonedVoice, Pcm, SynthRequest, Synthesized, VoiceBackend, VoiceError, VoiceSample, WordTiming,
};
pub use resample::{resample, Resampler};

pub mod estimator;
pub mod null;
pub mod registry;
pub mod stretch;
pub mod takes;

pub use estimator::{WpmEstimator, DEFAULT_WPM};
pub use null::{NullVoice, NULL_SAMPLE_RATE};
pub use registry::VoiceRegistry;

/// A voice teleprompt ships, by the name `voice.backend` gives it, and how
/// it is built from its `[backends.<id>]` settings (`None` when the project
/// has none). A build that fails is kept, and reported only where that
/// voice is chosen.
pub struct Provider {
    pub id: &'static str,
    pub build: Build,
}

/// How a provider is built from its settings: the voice, or why its
/// settings do not make one.
pub type Build = fn(Option<&serde_yaml::Value>) -> Result<std::sync::Arc<dyn VoiceBackend>, String>;
