//! The Gemini voice backend: Google's hosted text-to-speech models, Gemini
//! 3.8 Flash TTS and Flash-Lite TTS, through the Interactions API. The
//! narration is sent to Google, and the author's own key pays for it.
//! Like the other backends, written against `teleprompt-plugin` alone.

mod backend;
mod client;
mod config;

pub use backend::{plugin, GeminiVoice};
pub use config::GeminiConfig;
