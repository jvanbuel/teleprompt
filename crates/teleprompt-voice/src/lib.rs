pub mod contract;
pub mod null;
pub mod source;

pub use contract::{
    LanguageSupport, Pcm, SynthRequest, SynthResult, VoiceBackend, VoiceCapabilities, VoiceError,
    WordTiming,
};
pub use null::{estimate_ms, NullVoice, NULL_SAMPLE_RATE};
pub use source::{resolve_source, Resolution, VoiceSource};
