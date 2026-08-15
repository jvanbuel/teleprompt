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
