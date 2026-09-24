/// Re-exported so a backend uses the same macro version as
/// [`VoiceBackend`]; a mismatched copy fails with a confusing type error.
pub use async_trait::async_trait;

pub mod contract;
pub mod estimator;
pub mod registry;
pub mod source;
pub mod wav;

pub use contract::{
    LanguageSupport, Pcm, SynthRequest, Synthesized, VoiceBackend, VoiceCapabilities, VoiceError,
    WordTiming,
};
pub use estimator::DurationEstimator;
pub use registry::VoiceRegistry;
pub use source::{resolve_source, Resolution, VoiceSource};
