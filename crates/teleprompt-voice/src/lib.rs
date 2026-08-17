/// Re-exported so an out-of-tree backend can write `#[async_trait]` without
/// taking the dependency itself and keeping its version in step with ours.
/// `VoiceBackend` is an `async_trait` trait, so an implementor that used a
/// mismatched copy of the macro would not implement it at all — the failure
/// is a confusing type error, not a version warning.
pub use async_trait::async_trait;

pub mod contract;
pub mod estimator;
pub mod registry;
pub mod retry;
pub mod source;
pub mod wav;

pub use contract::{
    ErrorKind, LanguageSupport, Pcm, SynthRequest, Synthesized, VoiceBackend, VoiceCapabilities,
    VoiceError, WordTiming,
};
pub use estimator::DurationEstimator;
pub use registry::VoiceRegistry;
pub use retry::{with_retry, RetryPolicy};
pub use source::{resolve_source, Resolution, VoiceSource};
