//! What a scene plugin implements, in one crate: the contracts teleprompt's
//! scene plugins are written against, and the helpers they share
//! (`docs/guide/scene-plugins.md`).
//!
//! - [`scene`]: how a block of a scene plugin's own language compiles into
//!   shots, offline, so `plan` needs no tool.
//! - [`capture`]: how a scene's shots are recorded into clips.
//! - [`record`]: how an author's working session is recorded, for a scene
//!   plugin whose tool can.
//! - [`tool`]: running and finding the programs a plugin needs.
//! - [`protocol`]: a plugin as a program of its own, which teleprompt
//!   finds and talks to; and serving a Rust plugin as one.
//!
//! A scene plugin hands teleprompt one [`ScenePlugin`]: a scene language
//! and a way to record it. Voices are not plugins: a voice speaks OpenAI's
//! speech API (`teleprompt-voice`). Nothing here knows any plugin by name.

pub mod capture;
pub mod dirs;
pub mod protocol;
pub mod record;
pub mod scene;
mod scene_plugin;
pub mod tool;

pub use scene_plugin::{ScenePlugin, ScenePlugins};
