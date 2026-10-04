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
    LanguageSupport, Pcm, SynthRequest, Synthesized, VoiceBackend, VoiceCapabilities, VoiceError,
    WordTiming,
};
pub use resample::{resample, Resampler};
