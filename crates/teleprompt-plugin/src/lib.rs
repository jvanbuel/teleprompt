//! What a plugin implements, in one crate: the contracts teleprompt's
//! adapters and voices are written against, and the helpers they share
//! (`docs/guide/plugins.md`).
//!
//! - [`scene`]: how a block of an adapter's own language compiles into
//!   shots, offline, so `plan` needs no tool.
//! - [`capture`]: how a scene's shots are recorded into clips.
//! - [`record`]: how an author's working session is recorded, for an
//!   adapter whose tool can.
//! - [`voice`]: how a line is spoken.
//! - [`tool`]: running and finding the programs a plugin needs.
//! - [`protocol`]: a plugin as a program of its own, which teleprompt
//!   finds and talks to; and serving a Rust plugin as one.
//!
//! The two kinds of plugin are different things that share only the
//! plumbing: [`tool`] and [`protocol`]. An adapter hands teleprompt one
//! [`Adapter`], a scene language and a way to record it; a voice, one
//! [`VoicePlugin`], which builds a [`voice::VoiceBackend`] from its
//! settings. Nothing here knows any plugin by name.

mod adapter;
pub mod capture;
pub mod protocol;
pub mod record;
pub mod scene;
pub mod tool;
pub mod voice;

pub use adapter::Adapter;
pub use voice::VoicePlugin;
