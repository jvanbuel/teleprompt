//! The Gemini voice backend: Google's hosted text-to-speech models, Gemini
//! 3.8 Flash TTS and Flash-Lite TTS, through the Interactions API. The
//! narration is sent to Google, and the author's own key pays for it.

mod backend;
mod client;
mod config;

pub use backend::{provider, GeminiVoice};
pub use config::GeminiConfig;
