//! The ElevenLabs voice backend: its hosted text-to-speech models, spoken
//! in a premade voice or one of the author's own. The narration is sent to
//! ElevenLabs, and the author's own key pays for it. ElevenLabs does not
//! speak OpenAI's speech API, so it has a backend of its own. Like the
//! others, written against `teleprompt-plugin` alone.

mod backend;
mod client;
mod config;
pub mod tools;

pub use backend::{plugin, ElevenLabsVoice, DEFAULT_VOICE};
pub use config::ElevenLabsConfig;
