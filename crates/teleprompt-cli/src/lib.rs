//! The `teleprompt` command: each command's flags, what it does with the
//! crates below it, and how it reports. A command's flags and its `run`
//! are in its module under `commands`; what they share is `cli`. A
//! library so the tests can reach the commands; the binary is `main`.

pub mod ask;
pub mod cli;
pub mod commands;
pub mod output;
