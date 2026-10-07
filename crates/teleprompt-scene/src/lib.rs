//! What a scene plugin implements, in one crate: the contracts teleprompt's
//! scene plugins are written against, and the helpers they share
//! (`docs/guide/scene-plugins.md`).
//!
//! - [`contract`]: how a block of a scene plugin's own language compiles
//!   into shots, offline, so `plan` needs no tool. Re-exported at the root.
//! - [`capture`]: how a scene's shots are recorded into clips.
//! - [`record`]: how an author's working session is recorded, for a scene
//!   plugin whose tool can.
//! - [`core::tool`]: running and finding the programs a plugin needs,
//!   which is `core`'s, re-exported here.
//! - [`protocol`]: a plugin as a program of its own, which teleprompt
//!   finds and talks to; and serving a Rust plugin as one.
//!
//! A scene plugin hands teleprompt one [`ScenePlugin`]: a scene language
//! and a way to record it. Voices are not plugins: a voice speaks OpenAI's
//! speech API (`teleprompt-voice`). Nothing here knows any plugin by name.

pub mod capture;
pub mod contract;
pub mod dirs;
pub mod mock;
pub mod protocol;
pub mod record;
mod scene_plugin;

pub use contract::{
    is_content, select_marked, split_at_mark, validate_commands, validate_parts, BlockSource,
    BodyOrigin, CommandError, Measured, SceneCompiler, Shot, Validated,
};
pub use mock::MockScene;
pub use scene_plugin::{ScenePlugin, ScenePlugins};
/// The ids, diagnostics, hashes, times and settings a plugin speaks in.
pub use teleprompt_core as core;

/// The scene compilers a script can use, by the name a block's scene
/// gives: what the compiler reads a block's shots from.
pub trait SceneCompilers {
    /// The compiler named `name`, if there is one.
    fn compiler(&self, name: &str) -> Option<&dyn SceneCompiler>;
    /// Every name, in the order errors list them.
    fn names(&self) -> Vec<&'static str>;
}
