//! teleprompt's engine: a project's scripts compiled, dubbed, captured and
//! built, the prompter that follows a reader, and the drafts made from what
//! you have. The `teleprompt` command (`teleprompt-cli`) is its front end:
//! arguments, terminal output and exit codes.

pub mod build;
pub mod cache;
pub mod capture;
pub mod check;
pub mod draft;
pub mod dub;
pub mod edit;
pub mod error;
pub mod lsp;
pub mod new;
pub mod plan;
pub mod progress;
pub mod project;
pub mod registry;
pub mod serve;
pub mod setup;
pub mod translate;
pub mod voice;

pub use error::Failure;
