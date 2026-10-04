//! The Linux app around the prompter page: launching `teleprompt serve`,
//! the models setup installed, setting teleprompt up, and the tools a
//! session records with. Free of GTK; the window is in the binary.

pub mod launch;
pub mod models;
pub mod setup;
pub mod tools;
