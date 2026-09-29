//! The Voicebox voice backend: a local voice studio the author runs
//! (https://github.com/jamiepine/voicebox), speaking in a voice cloned from
//! their own takes or one designed from a description. teleprompt ships
//! none of it and runs no Python (docs/design.md#what-teleprompt-ships).
//! Like the other backends, written against `teleprompt-voice` alone.

mod backend;
mod client;
mod config;

pub use backend::VoiceboxVoice;
pub use client::{Profile, Sample};
pub use config::VoiceboxConfig;
