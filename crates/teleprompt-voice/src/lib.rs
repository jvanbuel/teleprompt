pub mod contract;
pub mod null;
pub mod source;

pub use contract::{
    LanguageSupport, SynthRequest, SynthResult, VoiceBackend, VoiceCapabilities, VoiceError,
    WordTiming,
};
pub use null::{estimate_ms, NullVoice};
pub use source::{resolve_source, Resolution, VoiceSource};
