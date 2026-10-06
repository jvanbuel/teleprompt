//! A teleprompt project on disk, and what the commands do to its scripts:
//! checked, planned, dubbed, captured, built, edited and translated, with
//! the caches that make a second run cheap. It is handed the scene plugins
//! and voices it works with as a [`Registry`](registry::Registry), and
//! knows none by name. The prompter, the drafts, setup and the language
//! server are crates on top of it; the `teleprompt` command
//! (`teleprompt-cli`) composes them all.

pub mod build;
pub mod cache;
pub mod capture;
pub mod check;
pub mod dub;
pub mod edit;
pub mod error;
pub mod new;
pub mod plan;
pub mod progress;
pub mod project;
pub mod registry;
pub mod translate;
pub mod voice;

pub use error::Failure;
