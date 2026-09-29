//! A native Linux prompter for teleprompt. Everything here but `mic` is
//! free of GTK: the API, launching `teleprompt prompt`, the session with
//! it, the prompter's state, the timeline and what a drag on it edits,
//! capturing and building, and the tools a session records with. The window is in the binary.

pub mod api;
pub mod launch;
pub mod make;
pub mod mic;
pub mod models;
pub mod retake;
pub mod ribbons;
pub mod said;
pub mod session;
pub mod state;
pub mod take;
pub mod timeline;
pub mod tools;
