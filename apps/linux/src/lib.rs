//! A native Linux prompter for teleprompt. Everything here but `mic` is
//! free of GTK: the API, launching `teleprompt prompt`, the session with
//! it, and the prompter's state. The window is in the binary.

pub mod api;
pub mod draft;
pub mod launch;
pub mod mic;
pub mod session;
pub mod state;
pub mod terminal;
