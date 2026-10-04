//! Voices from any server that speaks OpenAI's speech API.
//!
//! Most speech servers now do: Kokoro-FastAPI, OpenAI's own, and many a
//! wrapper around a local model. So one backend speaks to them all, with
//! presets for the two teleprompt names (`kokoro`, `openai`) and any other
//! under a name of the author's (`[backends.<name>]`, with its `base_url`).
//! teleprompt owns no Python and runs no model.

mod backend;
mod client;
mod config;

pub use backend::{endpoint, kokoro, openai, OpenAiVoice};
pub use config::{OpenAiConfig, Preset, PCM_SAMPLE_RATE};
