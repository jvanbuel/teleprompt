//! Voices from any server that speaks OpenAI's speech API.
//!
//! Most speech servers now do: Kokoro-FastAPI, OpenAI's own, and many a
//! wrapper around a local model. So one backend speaks to them all, with
//! presets for the two teleprompt names (`kokoro`, `openai`) and any other
//! under a name of the author's (`[backends.<name>] api = "openai"`).
//! teleprompt owns no Python and runs no model.
//!
//! Like the other backends, this crate is written against
//! `teleprompt-plugin`'s public API alone — it does not depend on
//! `teleprompt-core`. That is the standing proof that the contract admits a
//! backend nothing in it was designed around.

mod backend;
mod client;
mod config;
pub mod tools;

pub use backend::{endpoint, kokoro, openai, OpenAiVoice};
pub use config::{OpenAiConfig, Preset, PCM_SAMPLE_RATE};
