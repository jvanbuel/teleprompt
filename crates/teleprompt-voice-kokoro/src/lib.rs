//! The Kokoro-FastAPI voice backend.
//!
//! teleprompt speaks HTTP to a server the author runs and owns no Python.
//! Like the other backends, this crate is written against
//! `teleprompt-plugin`'s public API alone — it does not depend on
//! `teleprompt-core`. That is the standing proof that the contract admits a
//! backend nothing in it was designed around.

mod backend;
mod client;
mod config;
pub mod tools;

pub use backend::{plugin, KokoroVoice};
pub use config::{KokoroConfig, KOKORO_SAMPLE_RATE};
