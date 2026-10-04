//! How a line is spoken: the contract a voice backend implements
//! (`docs/design.md#voice-contract`), and the audio it hands back.

pub use async_trait::async_trait;
/// Re-exported so a backend uses the same macro version as
/// [`VoiceBackend`]; a mismatched copy fails with a confusing type error.
pub use teleprompt_core::error::with_causes;

mod contract;
mod resample;
pub mod wav;

pub use contract::{
    ClonedVoice, LanguageSupport, Pcm, SynthRequest, Synthesized, VoiceBackend, VoiceCapabilities,
    VoiceError, VoiceSample, WordTiming,
};
pub use resample::{resample, Resampler};

use std::sync::Arc;

/// A voice backend as teleprompt registers it: its id, which
/// `voice.backend` names, and how it is built from its own
/// `[backends.<id>]` settings (`None` when the project has none). A build
/// that fails is kept, and reported only where that backend is chosen.
pub struct VoicePlugin {
    pub id: &'static str,
    pub build: Build,
    /// What it needs that teleprompt does not ship: its server, say.
    pub needs: &'static [&'static crate::tool::Tool],
}

/// How a voice plugin is built from its settings: the backend, or why its
/// settings do not make one.
pub type Build = fn(Option<&serde_yaml::Value>) -> Result<Arc<dyn VoiceBackend>, String>;
