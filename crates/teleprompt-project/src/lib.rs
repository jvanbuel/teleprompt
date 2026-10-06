//! A teleprompt project on disk and its scripts: compiled, dubbed,
//! captured, built, edited and translated, with the caches that make a
//! second run cheap. What a command prints, and the commands that only
//! read (`check`, `plan`) or only write files (`new`), are the command
//! line's. It is handed the scene plugins
//! and voices it works with as a [`Registry`](registry::Registry), and
//! knows none by name. The prompter, the drafts, setup and the language
//! server are crates on top of it; the `teleprompt` command
//! (`teleprompt-cli`) composes them all.

pub mod build;
pub mod cache;
pub mod capture;
pub mod dub;
pub mod edit;
pub mod error;
pub mod project;
pub mod registry;
pub mod translate;
pub mod voice;

pub use error::Failure;
/// What a compiled script is made of, for whoever is handed one: so a
/// crate on top of this one need not name the pipeline crates itself.
pub use teleprompt_compile::{CompileOutput, NarrationDetail};
pub use teleprompt_manifest::NarrationManifest;
pub use teleprompt_schedule::Timeline;
