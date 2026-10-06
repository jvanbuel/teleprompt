//! The ElevenLabs voice backend: its hosted text-to-speech models, spoken
//! in a premade voice or one of the author's own. The narration is sent to
//! ElevenLabs, and the author's own key pays for it. ElevenLabs does not
//! speak OpenAI's speech API, so it has a backend of its own.

mod backend;
mod client;
mod config;

pub use backend::{provider, ElevenLabsVoice, DEFAULT_VOICE};
pub use config::ElevenLabsConfig;
