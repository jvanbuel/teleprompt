//! A native Linux prompter for teleprompt. Everything here but `mic` is
//! free of GTK: the API, launching `teleprompt prompt`, the session with
//! it, the prompter's state, and the tools a session records with. The
//! window is in the binary.

pub mod api;
pub mod launch;
pub mod mic;
pub mod session;
pub mod state;
pub mod tools;
